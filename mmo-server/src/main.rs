//! Test with:
//!
//!     nc -4u localhost 8080

extern crate clap;
extern crate futures;
#[macro_use]
extern crate tokio_core;

use clap::{Arg, App};

use std::{io};
use std::net::SocketAddr;
use std::collections::HashSet;

use futures::{Future, Poll};
use tokio_core::net::UdpSocket;
use tokio_core::reactor::Core;

struct Server {
    socket: UdpSocket,
    buf: Vec<u8>,
    peers: HashSet<SocketAddr>,
    to_send: Option<(usize, SocketAddr)>,
}

impl Future for Server {
    type Item = ();
    type Error = io::Error;

    fn poll(&mut self) -> Poll<(), io::Error> {
        loop {

            if let Some((size, peer)) = self.to_send {
                for recv in self.peers.iter().filter(|&&x| x != peer) {
                    try_nb!(self.socket.send_to(&self.buf[..size], recv));
                    // let amt = try_nb!(self.socket.send_to(&self.buf[..size], recv));
                    // println!("Echoed {}/{} bytes to {}", amt, size, peer);
                }
                self.to_send = None;
            }

            let (size, peer) = try_nb!(self.socket.recv_from(&mut self.buf));
            self.peers.insert(peer);
            
            self.to_send = Some((size, peer));
            
            // if let Some((size, peer)) = self.to_send {
            //     //let sparkle_heart = String::from_utf8(self.buf).unwrap();
            //     // println!("got: {:?} from: {}", self.buf, peer);
            // }
        }
    }
}

fn main() {

    let matches =
        App::new("mmo-server")
            .version("0.1.0")
            .about("Simulates a universe!")
            .author("Alex Rozgo")
            .arg(Arg::with_name("addr")
                .short("a")
                .long("address")
                .help("Host to connect to address:port")
                .takes_value(true))
            .get_matches();

    let addr = matches.value_of("addr").unwrap_or("127.0.0.1:8080");
    let addr = addr.parse::<SocketAddr>().unwrap();

    let mut l = Core::new().unwrap();
    let handle = l.handle();
    let socket = UdpSocket::bind(&addr, &handle).unwrap();
    println!("Listening on: {}", addr);

    l.run(Server {
        socket: socket,
        buf: vec![0; 1024],
        peers: HashSet::new(),
        to_send: None,
    }).unwrap();
}

