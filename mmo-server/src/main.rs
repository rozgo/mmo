//! Test with:
//!
//!     nc -4u localhost 8080

#[macro_use]
extern crate clap;
extern crate futures;
#[macro_use]
extern crate tokio_core;

use clap::{Arg, App};

use std::{io};
use std::net::SocketAddr;
use std::collections::HashMap;
use std::time::{Instant, Duration};

use futures::{Future, Poll};
use tokio_core::net::UdpSocket;
use tokio_core::reactor::Core;

struct Client {
    instant: Instant,
}

struct Server {
    socket: UdpSocket,
    buf: Vec<u8>,
    clients: HashMap<SocketAddr, Client>,
    sender: Option<(usize, SocketAddr)>,
    expiration: Duration,
}

impl Future for Server {
    type Item = ();
    type Error = io::Error;

    fn poll(&mut self) -> Poll<(), io::Error> {
        loop {

            let mut expired = Vec::new();

            if let Some((size, peer)) = self.sender {
                for recv in self.clients.keys().filter(|&&x| x != peer) {
                    try_nb!(self.socket.send_to(&self.buf[..size], recv));
                    if let Some(client) = self.clients.get(recv) {
                        if client.instant.elapsed() > self.expiration {
                            expired.push(recv.clone());
                            println!("Expired: {}", recv);
                        }
                    }
                }
                self.sender = None;
            }

            for peer in expired {
                self.clients.remove(&peer);
            }
            
            let (size, peer) = try_nb!(self.socket.recv_from(&mut self.buf));
            self.clients.insert(peer, Client{instant: Instant::now()});
            self.sender = Some((size, peer));
        }
    }
}

fn main() {

    let matches =
        App::new("mmo-server")
            .version("0.1.0")
            .about("Simulates a slice of universe!")
            .author("Alex Rozgo")
            .arg(Arg::with_name("addr")
                .short("a")
                .long("address")
                .help("Host to connect to address:port")
                .takes_value(true))
            .arg(Arg::with_name("exp")
                .short("e")
                .long("expiration")
                .help("Connection expiration limit")
                .takes_value(true))
            .get_matches();

    let addr = matches.value_of("addr").unwrap_or("127.0.0.1:8080");
    let addr = addr.parse::<SocketAddr>().unwrap();

    let exp = value_t!(matches, "exp", u64).unwrap_or(10);

    let mut l = Core::new().unwrap();
    let handle = l.handle();
    let socket = UdpSocket::bind(&addr, &handle).unwrap();
    println!("Listening on: {}", addr);

    l.run(Server {
        socket: socket,
        buf: vec![0; 1024],
        clients: HashMap::new(),
        sender: None,
        expiration: Duration::from_secs(exp),
    }).unwrap();
}




            // if let Some((size, peer)) = self.to_send {
            //     //let sparkle_heart = String::from_utf8(self.buf).unwrap();
            //     // println!("got: {:?} from: {}", self.buf, peer);
            // }


                    // let amt = try_nb!(self.socket.send_to(&self.buf[..size], recv));
                    // println!("Echoed {}/{} bytes to {}", amt, size, peer);