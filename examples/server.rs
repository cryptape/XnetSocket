#[allow(unused_imports)]
extern crate XnetSocket;
#[macro_use]
extern crate log;
extern crate env_logger;

use XnetSocket::listen;

fn main() {

    env_logger::init().unwrap();

    if let Err(error) = listen("127.0.0.1:3012".to_string(), |out| {
        move |msg| {
            info!("Server got message '{}'. ", msg);
            //			Ok(())
            out.send(msg)
        }
    })
    {
        println!("Failed to create Socket due to {:?}", error);
    }
}
