extern crate futures;
extern crate futures_io;
extern crate futures_mio;

use std::net::SocketAddr;

use futures::Future;
use futures_io::{copy, TaskIo};
use futures::stream::Stream;

fn main() {
    let addr = "127.0.0.1:8080".parse::<SocketAddr>().unwrap();

    let mut l = futures_mio::Loop::new().unwrap();

    let server = l.handle().tcp_listen(&addr);

    let done = server.and_then(move |socket| {
        println!("Listening on: {}", addr);

        socket.incoming().for_each(|(socket, addr)| {
            let io = TaskIo::new(socket);
            let pair = io.map(|io| io.split());
            let amt = pair.and_then(|(reader, writer)| {
                copy(reader, writer)
            });
            amt.map(move |amt| {
                println!("wrote {} bytes to {}", amt, addr)
            }).forget();

            Ok(())
        })
    });
    l.run(done).unwrap();
}


