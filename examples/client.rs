extern crate XnetSocket;
#[macro_use]
extern crate log;
extern crate env_logger;

use XnetSocket::{connect, CloseCode};

fn main() {

    env_logger::init().unwrap();
    if let Err(error) = connect("127.0.0.1:3012".to_string(), |out| {
        if let Err(_) = out.send("Hello") {
            println!("socket couldn't queue an initial message.")
        } else {
            println!("Client sent message 'Hello Socket'. ")
        }
        move |msg| {
            println!("Client got message '{}'. ", msg);
            out.close(CloseCode::Normal)
        }
    })
    {

        println!("Failed to create Socket due to: {:?}", error);
    }
}
