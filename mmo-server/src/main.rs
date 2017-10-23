#[macro_use]
extern crate clap;
extern crate futures;
extern crate tokio_core;

use clap::{Arg, App};

use std::io;
use std::io::{Error, ErrorKind};
use std::net::SocketAddr;
use std::collections::HashMap;
use std::time::{Instant, Duration};

use futures::{Future, stream, Stream, Sink};
use tokio_core::net::{UdpSocket, UdpCodec};
use tokio_core::reactor::Core;

pub struct LineCodec;

impl UdpCodec for LineCodec {
    type In = (SocketAddr, Vec<u8>);
    type Out = (SocketAddr, Vec<u8>);

    fn decode(&mut self, addr: &SocketAddr, buf: &[u8]) -> io::Result<Self::In> {
        Ok((*addr, buf.to_vec()))
    }

    fn encode(&mut self, (addr, buf): Self::Out, into: &mut Vec<u8>) -> SocketAddr {
        into.extend(buf);
        addr
    }
}

pub struct Client {
    pub instant: Instant,
}

fn main() {

    let matches = App::new("mmo-server")
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
    let exp = value_t!(matches, "exp", u64).unwrap_or(5);

    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let udp_local_addr: SocketAddr = addr;
    let server_socket = UdpSocket::bind(&udp_local_addr, &handle).unwrap();
    println!("Listening on: {}", addr);
    let (udp_socket_tx, udp_socket_rx) = server_socket.framed(LineCodec).split();
    let expiration = Duration::from_secs(exp);
    let clients = &mut HashMap::<SocketAddr, Client>::new();

    let listen_task = udp_socket_rx.fold((clients, udp_socket_tx), |(clients, tx), (client_socket, msg)| {

        let mut expired = Vec::new();
        for (socket, client) in clients.iter() {
            if client.instant.elapsed() > expiration {
                expired.push(socket.clone());
                println!("Expired: {} {}", expired.len(), socket);
            }
        }

        for client_socket in expired {
            clients.remove(&client_socket);
            println!("Removed: {} Online: {}", client_socket, clients.len());
        }

        if !clients.contains_key(&client_socket) {
            println!("Connected: {}", client_socket);
        }

        clients.insert(client_socket, Client { instant: Instant::now() });

        let client_sockets: Vec<_> = clients.keys()
            .filter(|&&x| x != client_socket)
            .map(|k| k.clone()).collect();

        stream::iter_ok::<_, ()>(client_sockets)
        .fold(tx, move |tx, client_socket| {
            tx.send((client_socket.clone(), msg.clone()))
            .map_err(|_| ())
        })
        .map(|a| (clients, a))
        .map_err(|_| Error::new(ErrorKind::Other, "broadcasting to clients"))
    });

    if let Err(err) = core.run(listen_task) {
        println!("{}", err);
    }
}
