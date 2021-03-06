use super::Settings;
use communication::{Sender, Signal, Command};
use connection::Connection;
use factory::Factory;
use mio;
use mio::{Token, Ready, Poll, PollOpt};
use mio::tcp::{TcpListener, TcpStream};
use result::{Result, Error, Kind};
use std::borrow::Borrow;
use std::io::{ErrorKind, Error as IoError};
use std::net::{SocketAddr, ToSocketAddrs};
use std::time::Duration;
use std::usize;
use url::Url;
use util::Slab;
const QUEUE: Token = Token(usize::MAX - 3); //接受数据方监听的fd,
const TIMER: Token = Token(usize::MAX - 4);
pub const ALL: Token = Token(usize::MAX - 5);
const SYSTEM: Token = Token(usize::MAX - 6);

type Conn<F> = Connection<<F as Factory>::Handler>;

const MAX_EVENTS: usize = 1024;
const MESSAGES_PER_TICK: usize = 256;
const TIMER_TICK_MILLIS: u64 = 100;
const TIMER_WHEEL_SIZE: usize = 1024;
const TIMER_CAPACITY: usize = 65_536;

#[cfg(not(windows))]
const CONNECTION_REFUSED: i32 = 111;
#[cfg(windows)]
const CONNECTION_REFUSED: i32 = 61;

fn url_to_addrs(url: &String) -> Result<Vec<SocketAddr>> {
    //    let host = url.host_str();

    //    if host.is_none() || (url.scheme() != "ws" && url.scheme() != "wss") {
    //        return Err(Error::new(Kind::Internal, format!("Not a valid socket url: {}", url)));
    //    }
    //    let host = host.unwrap();
    //
    //    let port = url.port_or_known_default().unwrap_or(80);
    //    let mut addrs = try!((&host[..], port).to_socket_addrs()).collect::<Vec<SocketAddr>>();
    //    addrs.dedup();

    //TODO TCP address
    let addrs = Vec::from(url.to_socket_addrs().unwrap().as_slice());
    Ok(addrs)
}

enum State {
    Active,
    Inactive,
}

impl State {
    fn is_active(&self) -> bool {
        match *self {
            State::Active => true,
            State::Inactive => false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Timeout {
    connection: Token,
    event: Token,
}

pub struct Handler<F>
where
    F: Factory,
{
    listener: Option<TcpListener>,
    connections: Slab<Conn<F>>,
    factory: F,
    settings: Settings,
    state: State,
    queue_tx: mio::channel::SyncSender<Command>,
    queue_rx: mio::channel::Receiver<Command>,
    timer: mio::timer::Timer<Timeout>,
    next_connection_id: u32,
}


impl<F> Handler<F>
where
    F: Factory,
{
    pub fn new(factory: F, settings: Settings) -> Handler<F> {
        let (tx, rx) = mio::channel::sync_channel(settings.max_connections * settings.queue_size);
        let timer = mio::timer::Builder::default()
            .tick_duration(Duration::from_millis(TIMER_TICK_MILLIS))
            .num_slots(TIMER_WHEEL_SIZE)
            .capacity(TIMER_CAPACITY)
            .build();
        Handler {
            listener: None,
            connections: Slab::with_capacity(settings.max_connections),
            factory: factory,
            settings: settings,
            state: State::Inactive,
            queue_tx: tx,
            queue_rx: rx,
            timer: timer,
            next_connection_id: 0,
        }
    }

    pub fn sender(&self) -> Sender {
        Sender::new(ALL, self.queue_tx.clone(), 0)
    }

    pub fn sender_handler(&self) -> mio::channel::SyncSender<Command> {
        self.queue_tx.clone()
    }

    pub fn listen(&mut self, poll: &mut Poll, addr: &SocketAddr) -> Result<&mut Handler<F>> {
        debug_assert!(self.listener.is_none(), "Attempted to listen for connections from two addresses on the same socket.");

        let tcp = TcpListener::bind(addr)?;
        // TODO: consider net2 in order to set reuse_addr
        poll.register(&tcp, ALL, Ready::readable(), PollOpt::level())?;
        self.listener = Some(tcp);
        Ok(self)
    }

    pub fn local_addr(&self) -> ::std::io::Result<SocketAddr> {
        if let Some(ref listener) = self.listener {
            listener.local_addr()
        } else {
            Err(IoError::new(ErrorKind::NotFound, "Not a listening socket"))
        }
    }


    pub fn connect(&mut self, poll: &mut Poll, url: String) -> Result<()> {
        let settings = self.settings;

        let (tok, addresses) = {
            let (tok, entry, connection_id, handler) = if let Some(entry) = self.connections.vacant_entry() {
                let tok = entry.index();
                let connection_id = self.next_connection_id;
                self.next_connection_id = self.next_connection_id.wrapping_add(1);
                (tok, entry, connection_id, self.factory.client_connected(Sender::new(tok, self.queue_tx.clone(), connection_id)))
            } else {
                return Err(Error::new(Kind::Capacity, "Unable to add another connection to the event loop."));
            };

            let mut addresses = match url_to_addrs(&url) {
                Ok(addresses) => addresses,
                Err(err) => {
                    self.factory.connection_lost(handler);
                    return Err(err);
                }
            };

            loop {
                if let Some(addr) = addresses.pop() {
                    if let Ok(sock) = TcpStream::connect(&addr) {
                        if settings.tcp_nodelay {
                            sock.set_nodelay(true)?
                        }
                        let mut conn = Connection::new(tok, sock, handler, settings, connection_id);
                        //TODO connected to do on_open() function
                        conn.open();
                        entry.insert(conn);
                        break;
                    }
                } else {
                    self.factory.connection_lost(handler);
                    return Err(Error::new(Kind::Internal, format!("Unable to obtain any socket address for {}", url)));
                }
            }

            (tok, addresses)
        };

        if let Err(error) = self.connections[tok].as_client(url, addresses) {
            let handler = self.connections.remove(tok).unwrap().consume();
            self.factory.connection_lost(handler);
            return Err(error);
        }

        //register socket event
        poll.register(self.connections[tok].socket(), self.connections[tok].token(), self.connections[tok].events(), PollOpt::edge() | PollOpt::oneshot())
            .map_err(Error::from)
            .or_else(|err| {
                         error!("Encountered error while trying to build socket connection: {}", err);
                         let handler = self.connections.remove(tok).unwrap().consume();
                         self.factory.connection_lost(handler);
                         Err(err)
                     })
    }


    pub fn accept(&mut self, poll: &mut Poll, sock: TcpStream) -> Result<()> {
        let factory = &mut self.factory;
        let settings = self.settings;

        if settings.tcp_nodelay {
            sock.set_nodelay(true)?
        }

        let tok = {
            if let Some(entry) = self.connections.vacant_entry() {
                let tok = entry.index();
                let connection_id = self.next_connection_id;
                self.next_connection_id = self.next_connection_id.wrapping_add(1);
                let handler = factory.server_connected(Sender::new(tok, self.queue_tx.clone(), connection_id));
                entry.insert(Connection::new(tok, sock, handler, settings, connection_id));
                tok
            } else {
                return Err(Error::new(Kind::Capacity, "Unable to add another connection to the event loop."));
            }
        };

        let conn = &mut self.connections[tok];
        conn.as_server()?; //监听可读

        let ret: Result<()> = poll.register(conn.socket(), conn.token(), conn.events(), PollOpt::edge() | PollOpt::oneshot())
                                  .map_err(|err| Error::from(err))
                                  .or_else(|err| {
                                               error!("Encountered error while trying to build socket connection: {}", err);
                                               conn.error(err);
                                               if settings.panic_on_new_connection {
                                                   panic!("Encountered error while trying to build socket connection.");
                                               }
                                               Ok(())
                                           });
        ret?;

        //open connection on_open() to change state
        trace!("acecept new connection");
        conn.open().map_err(Error::from).or_else(|err| {
                                                     error!("Encountered error while trying to build socket connection: {}", err);
                                                     conn.error(err);
                                                     if settings.panic_on_new_connection {
                                                         panic!("Encountered error while trying to build socket connection.");
                                                     }
                                                     Ok(())
                                                 })
    }

    pub fn run(&mut self, poll: &mut Poll) -> Result<()> {
        trace!("Running event loop");
        poll.register(&self.queue_rx, QUEUE, Ready::readable(), PollOpt::edge() | PollOpt::oneshot())?;
        poll.register(&self.timer, TIMER, Ready::readable(), PollOpt::edge())?;

        self.state = State::Active;
        let result = self.event_loop(poll);

        //close XnetSocket after clean's work
        self.state = State::Inactive;
        result.and(poll.deregister(&self.timer).map_err(|e| Error::from(e)))
              .and(poll.deregister(&self.queue_rx).map_err(|e| Error::from(e)))
    }

    #[inline]
    fn event_loop(&mut self, poll: &mut Poll) -> Result<()> {
        let mut events = mio::Events::with_capacity(MAX_EVENTS);
        while self.state.is_active() {
            trace!("Waiting for event");
            let nevents = match poll.poll(&mut events, None) {
                //监听接收事件。
                Ok(nevents) => nevents,
                Err(err) => {
                    if err.kind() == ErrorKind::Interrupted {
                        if self.settings.shutdown_on_interrupt {
                            error!("socket shutting down for interrupt.");
                            self.state = State::Inactive;
                        } else {
                            error!("socket received interupt.");
                        }
                        0
                    } else {
                        return Err(Error::from(err));
                    }
                }
            };
            trace!("Processing {} events", nevents);

            for i in 0..nevents {
                let evt = events.get(i).unwrap();
                self.handle_event(poll, evt.token(), evt.kind());
            }

            self.check_count();
        }
        Ok(())
    }

    #[inline]
    fn schedule(&self, poll: &mut Poll, conn: &Conn<F>) -> Result<()> {
        trace!("Scheduling connection to {} as {:?}", conn.socket().peer_addr().map(|addr| addr.to_string()).unwrap_or("UNKNOWN".into()), conn.events());
        Ok(poll.reregister(conn.socket(), conn.token(), conn.events(), PollOpt::edge() | PollOpt::oneshot())?)
    }

    fn shutdown(&mut self) {
        debug!("Received shutdown signal. socket is attempting to shut down.");
        for conn in self.connections.iter_mut() {
            conn.shutdown();
        }
        self.factory.on_shutdown();
        self.state = State::Inactive;
        if self.settings.panic_on_shutdown {
            panic!("Panicking on shutdown as per setting.")
        }
    }

    #[inline]
    fn check_active(&mut self, poll: &mut Poll, active: bool, token: Token) {
        if !active {
            if let Ok(addr) = self.connections[token].socket().peer_addr() {
                debug!("socket connection to {} disconnected.", addr);
            } else {
                trace!("socket connection to token={:?} disconnected.", token);
            }
            let handler = self.connections.remove(token).unwrap().consume();
            self.factory.connection_lost(handler);
        } else {
            self.schedule(poll, &self.connections[token])
                .or_else(|err| {
                             // This will be an io error, so disconnect will already be called
                             self.connections[token].error(Error::from(err));
                             let handler = self.connections.remove(token).unwrap().consume();
                             self.factory.connection_lost(handler);
                             Ok::<(), Error>(())
                         })
                .unwrap()
        }
    }

    #[inline]
    fn is_client(&self) -> bool {
        self.listener.is_none()
    }

    #[inline]
    fn check_count(&mut self) {
        trace!("Active connections {:?}", self.connections.len());
        if self.connections.len() == 0 {
            if !self.state.is_active() {
                debug!("Shutting down socket server.");

            } else if self.is_client() {
                debug!("Shutting down socket client.");
                self.factory.on_shutdown();
                self.state = State::Inactive;
            }
        }
    }

    fn handle_event(&mut self, poll: &mut Poll, token: Token, events: Ready) {
        match token {
            SYSTEM => {
                debug_assert!(false, "System token used for io event. This is a bug!");
                error!("System token used for io event. This is a bug!");
            }
            ALL => {
                if events.is_readable() {
                    match self.listener.as_ref().expect("No listener provided for server socket connections").accept() {
                        Ok((sock, addr)) => {
                            info!("Accepted a new tcp connection from {}.", addr);
                            if let Err(err) = self.accept(poll, sock) {
                                error!("Unable to build socket connection {:?}", err);
                                if self.settings.panic_on_new_connection {
                                    panic!("Unable to build socket connection {:?}", err);
                                }
                            }
                        }
                        Err(err) => error!("Encountered an error {:?} while accepting tcp connection.", err),
                    }
                }
            }
            TIMER => {
                while let Some(t) = self.timer.poll() {
                    self.handle_timeout(poll, t);
                }
            }
            QUEUE => {
                //监听的队列事件发生，接受服务发的数据，服务socket数据都是通过chanel一起发的。
                for _ in 0..MESSAGES_PER_TICK {
                    match self.queue_rx.try_recv() {
                        Ok(cmd) => self.handle_queue(poll, cmd),
                        Err(err) => error!("message recive data queue error {:?}", err.to_string()),
                    };
                    break;
                }
                let _ = poll.reregister(&self.queue_rx, QUEUE, Ready::readable(), PollOpt::edge() | PollOpt::oneshot());
            }
            _ => {
                //监听的socket事件发生。
                let active = {
                    let conn_events = self.connections[token].events();
                    if (events & conn_events).is_readable() {
                        //可读
                        if let Err(err) = self.connections[token].read() {
                            //读数据，
                            trace!("Encountered error while reading: {}", err);
                            if let Kind::Io(ref err) = err.kind {
                                if let Some(errno) = err.raw_os_error() {
                                    if errno == CONNECTION_REFUSED {
                                        match self.connections[token].reset() {
                                            Ok(_) => {
                                                poll.register(self.connections[token].socket(), self.connections[token].token(), self.connections[token].events(), PollOpt::edge() | PollOpt::oneshot())
                                                    .or_else(|err| {
                                                                 self.connections[token].error(Error::from(err));
                                                                 let handler = self.connections.remove(token).unwrap().consume();
                                                                 self.factory.connection_lost(handler);
                                                                 Ok::<(), Error>(())
                                                             })
                                                    .unwrap();
                                                return;
                                            }
                                            Err(err) => {
                                                trace!("Encountered error while trying to reset connection: {:?}", err);
                                            }
                                        }
                                    }
                                }
                            }
                            // This will trigger disconnect if the connection is open
                            self.connections[token].error(err)
                        }
                    }

                    let conn_events = self.connections[token].events();

                    if (events & conn_events).is_writable() {
                        //可写
                        if let Err(err) = self.connections[token].write() {
                            //write data
                            trace!("Encountered error while writing: {}", err);
                            if let Kind::Io(ref err) = err.kind {
                                if let Some(errno) = err.raw_os_error() {
                                    if errno == CONNECTION_REFUSED {
                                        match self.connections[token].reset() {
                                            Ok(_) => {
                                                poll.register(self.connections[token].socket(), self.connections[token].token(), self.connections[token].events(), PollOpt::edge() | PollOpt::oneshot())
                                                    .or_else(|err| {
                                                                 self.connections[token].error(Error::from(err));
                                                                 let handler = self.connections.remove(token).unwrap().consume();
                                                                 self.factory.connection_lost(handler);
                                                                 Ok::<(), Error>(())
                                                             })
                                                    .unwrap();
                                                return;
                                            }
                                            Err(err) => {
                                                trace!("Encountered error while trying to reset connection: {:?}", err);
                                            }
                                        }
                                    }
                                }
                            }
                            // This will trigger disconnect if the connection is open
                            self.connections[token].error(err)
                        }
                    }

                    // connection events may have changed
                    self.connections[token].events().is_readable() || self.connections[token].events().is_writable()
                };

                self.check_active(poll, active, token)
            }
        }
    }

    fn handle_queue(&mut self, poll: &mut Poll, cmd: Command) {
        match cmd.token() {
            SYSTEM => {
                // Scaffolding for system events such as internal timeouts
            }
            ALL => {
                //broadcasting message with type
                let mut dead = Vec::with_capacity(self.connections.len());

                match cmd.signal() {
                    Signal::Message(msg) => {
                        trace!("Broadcasting message: {:?}", msg);
                        for conn in self.connections.iter_mut() {
                            if let Err(err) = conn.send_message(msg.clone()) {
                                dead.push((conn.token(), err))
                            }
                        }
                    }
                    Signal::Close(code, reason) => {
                        trace!("Broadcasting close: {:?} - {}", code, reason);
                        for conn in self.connections.iter_mut() {
                            if let Err(err) = conn.send_close(code, reason.borrow()) {
                                dead.push((conn.token(), err))
                            }
                        }
                    }

                    Signal::Connect(url) => {
                        if let Err(err) = self.connect(poll, url.clone()) {
                            if self.settings.panic_on_new_connection {
                                panic!("Unable to establish connection to {}: {:?}", url, err);
                            }
                            error!("Unable to establish connection to {}: {:?}", url, err);
                        }
                        return;
                    }
                    Signal::Shutdown => self.shutdown(),
                    Signal::Timeout { delay, token: event } => {
                        match self.timer
                                    .set_timeout(Duration::from_millis(delay), Timeout { connection: ALL, event: event })
                                    .map_err(Error::from) {
                            Ok(timeout) => {
                                for conn in self.connections.iter_mut() {
                                    if let Err(err) = conn.new_timeout(event, timeout.clone()) {
                                        conn.error(err)
                                    }
                                }
                            }
                            Err(err) => {
                                if self.settings.panic_on_timeout {
                                    panic!("Unable to schedule timeout: {:?}", err);
                                }
                                error!("Unable to schedule timeout: {:?}", err);
                            }
                        }
                        return;
                    }
                    Signal::Cancel(timeout) => {
                        self.timer.cancel_timeout(&timeout);
                        return;
                    }
                }

                for conn in self.connections.iter() {
                    if let Err(err) = self.schedule(poll, conn) {
                        dead.push((conn.token(), err))
                    }
                }
                for (token, err) in dead {
                    // note the same connection may be called twice
                    self.connections[token].error(err)
                }
            }

            token => {
                //single socket send message
                let connection_id = cmd.connection_id();
                match cmd.signal() {
                    Signal::Message(msg) => {
                        if let Some(conn) = self.connections.get_mut(token) {
                            if conn.connection_id() == connection_id {
                                if let Err(err) = conn.send_message(msg) {
                                    conn.error(err)
                                }
                            } else {
                                trace!("Connection disconnected while a message was waiting in the queue.")
                            }
                        } else {
                            trace!("Connection disconnected while a message was waiting in the queue.")
                        }
                    }
                    Signal::Close(code, reason) => {
                        if let Some(conn) = self.connections.get_mut(token) {
                            if conn.connection_id() == connection_id {
                                if let Err(err) = conn.send_close(code, reason) {
                                    conn.error(err)
                                }
                            } else {
                                trace!("Connection disconnected while close signal was waiting in the queue.")
                            }
                        } else {
                            trace!("Connection disconnected while close signal was waiting in the queue.")
                        }
                    }

                    Signal::Connect(url) => {
                        if let Err(err) = self.connect(poll, url.clone()) {
                            if let Some(conn) = self.connections.get_mut(token) {
                                conn.error(err)
                            } else {
                                if self.settings.panic_on_new_connection {
                                    panic!("Unable to establish connection to {}: {:?}", url, err);
                                }
                                error!("Unable to establish connection to {}: {:?}", url, err);
                            }
                        }
                        return;
                    }
                    Signal::Shutdown => self.shutdown(),
                    Signal::Timeout { delay, token: event } => {
                        match self.timer
                                    .set_timeout(Duration::from_millis(delay), Timeout { connection: token, event: event })
                                    .map_err(Error::from) {
                            Ok(timeout) => {
                                if let Some(conn) = self.connections.get_mut(token) {
                                    if let Err(err) = conn.new_timeout(event, timeout) {
                                        conn.error(err)
                                    }
                                } else {
                                    trace!("Connection disconnected while pong signal was waiting in the queue.")
                                }
                            }
                            Err(err) => {
                                if let Some(conn) = self.connections.get_mut(token) {
                                    conn.error(err)
                                } else {
                                    trace!("Connection disconnected while pong signal was waiting in the queue.")
                                }
                            }
                        }
                        return;
                    }
                    Signal::Cancel(timeout) => {
                        self.timer.cancel_timeout(&timeout);
                        return;
                    }
                }

                if let Some(_) = self.connections.get(token) {
                    if let Err(err) = self.schedule(poll, &self.connections[token]) {
                        self.connections[token].error(err)
                    }
                }
            }
        }
    }


    fn handle_timeout(&mut self, poll: &mut Poll, Timeout { connection, event }: Timeout) {
        let active = {
            if let Some(conn) = self.connections.get_mut(connection) {
                if let Err(err) = conn.timeout_triggered(event) {
                    conn.error(err)
                }

                conn.events().is_readable() || conn.events().is_writable()
            } else {
                trace!("Connection disconnected while timeout was waiting.");
                return;
            }
        };
        self.check_active(poll, active, connection);
    }
}
