use communication::Sender;
use handler::Handler;

/// A trait for creating new Socket handlers.
pub trait Factory {
    type Handler: Handler;

    /// Called when a TCP connection is made.
    fn connection_made(&mut self, _: Sender) -> Self::Handler;

    /// Called when the socket is shutting down.
    #[inline]
    fn on_shutdown(&mut self) {
        debug!("Factory received Socket shutdown request.");
    }


    #[inline]
    fn client_connected(&mut self, xnet: Sender) -> Self::Handler {
        self.connection_made(xnet)
    }


    #[inline]
    fn server_connected(&mut self, xnet: Sender) -> Self::Handler {
        self.connection_made(xnet)
    }


    #[inline]
    fn connection_lost(&mut self, _: Self::Handler) {}
}

impl<F, H> Factory for F
where
    H: Handler,
    F: FnMut(Sender) -> H,
{
    type Handler = H;

    fn connection_made(&mut self, out: Sender) -> H {
        self(out)
    }
}

mod test {
#![allow(unused_imports, unused_variables, dead_code)]
    use super::*;
    use communication::{Command, Sender};
    use handler::Handler;
    use message;
    use mio;
    use protocol::CloseCode;
    use result::Result;

    #[derive(Debug, Eq, PartialEq)]
    struct M;

    impl Handler for M {
        fn on_message(&mut self, _: message::Message) -> Result<()> {
            Ok(println!("test"))
        }
    }

    #[test]
    fn impl_factory() {
        struct X;

        impl Factory for X {
            type Handler = M;
            fn connection_made(&mut self, _: Sender) -> M {
                M
            }
        }

        let (chn, _) = mio::channel::sync_channel(42);

        let mut x = X;
        let m = x.connection_made(Sender::new(mio::Token(0), chn, 0));
        assert_eq!(m, M);
    }

    #[test]
    fn closure_factory() {
        let (chn, _) = mio::channel::sync_channel(42);

        let mut factory = |_| |_| Ok(());

        factory.connection_made(Sender::new(mio::Token(0), chn, 0));
    }

    #[test]
    fn connection_lost() {
        struct X;

        impl Factory for X {
            type Handler = M;
            fn connection_made(&mut self, _: Sender) -> M {
                M
            }
            fn connection_lost(&mut self, handler: M) {
                assert_eq!(handler, M);
            }
        }

        let (chn, _) = mio::channel::sync_channel(42);

        let mut x = X;
        let m = x.connection_made(Sender::new(mio::Token(0), chn, 0));
        x.connection_lost(m);
    }
}
