#[macro_use]
extern crate clap;
extern crate openssl;
extern crate tokio_core;
extern crate tokio_openssl;
extern crate tokio_io;
extern crate tokio_file_unix;
extern crate tokio_timer;
extern crate protobuf;
extern crate byteorder;

use clap::{Arg, App};extern crate futures;

use std::fs::File;
use std::io::{Write, Read, Error, ErrorKind};
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use futures::Sink;
use futures::future::{Future, ok, lazy, result, err, loop_fn, Loop, empty};
use futures::stream::{self, Stream};

use openssl::ssl::{self, SslContext, SslMethod, SSL_VERIFY_PEER};
use openssl::x509::X509_FILETYPE_PEM;

use tokio_io::io;
use tokio_io::AsyncRead;
use tokio_core::net::TcpStream;
use tokio_core::reactor::Core;
use tokio_timer::Timer;

use protobuf::Message;
use protobuf::{CodedOutputStream, CodedInputStream};

use std::io::Cursor;
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};

mod mumble;
mod connector;
use connector::MumbleConnector;

mod varint;
use varint::VarintReader;

fn err_str<T>(e : T) -> Error
where T: std::string::ToString
{
    Error::new(ErrorKind::Other, e.to_string())
}

fn main() {

    let matches = App::new("mmo-mumble")
        .version("0.1.0")
        .about("Voice client bot!")
        .author("Alex Rozgo")
        .arg(Arg::with_name("addr")
            .short("a")
            .long("address")
            .help("Host to connect to address:port")
            .takes_value(true))
        .get_matches();

    let addr_str = matches.value_of("addr").unwrap_or("127.0.0.1:8080");
    let addr = addr_str.parse::<SocketAddr>().unwrap();

    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let client = TcpStream::connect(&addr, &handle);

    let data = client.and_then(|socket| {

        let path = Path::new("mumble.pem");

        let mut ctx = SslContext::builder(SslMethod::tls()).unwrap();
        ctx.set_verify_callback(SSL_VERIFY_PEER, |_, _| true);

        assert!(ctx.set_certificate_file(&path, X509_FILETYPE_PEM).is_ok());

        let ctx = ctx.build();
        let connector = MumbleConnector(ctx);
        connector.connect_async(addr_str, socket).map_err(err_str)

    // }).and_then(|stream| {
    //     let version = mumble::Version::new();
    //     let s = version.compute_size();
    //     println!("version size: {}", s);
    //     let mut buf = vec![0u8; s as usize];
    //     {
    //     let buf = &mut buf;
    //     let os = &mut CodedOutputStream::vec(buf);
    //     assert!(os.flush().is_ok());
    //     assert!(version.write_to_with_cached_sizes(os).is_ok());
    //     }
    //     io::write_all(stream, buf)

    }).and_then(|stream| {
        let mut auth = mumble::Authenticate::new();
        auth.set_username("lyric".to_string());
        let s = auth.compute_size();
        println!("auth size: {}", s);
        let mut buf = vec![0u8; (s + 6) as usize];
        buf[1] = 2u8; // Packet type: Authenticate
        (&mut buf[2..]).write_u32::<BigEndian>(s).unwrap();
        {
        let os = &mut CodedOutputStream::bytes(&mut buf[6..]);
        assert!(os.flush().is_ok());
        assert!(auth.write_to_with_cached_sizes(os).is_ok());
        }
        println!("MSG {:?}", buf);
        io::write_all(stream, buf)

    }).and_then(|(stream, _)| {

        let file = File::create("dump.opus").unwrap();
        let file = tokio_file_unix::File::new_nb(file).unwrap();
        let (file_tx, rx) = futures::sync::mpsc::unbounded::<Vec<u8>>();
        let file_writer = rx.fold(file, move |mut writer, msg : Vec<u8>| {
            // println!("{:?}", msg);
            println!("dumping {:?}", msg.len());
            writer.write(&msg[..])
            .map(|_| writer)
            .map_err(|_| ())
        })
        .map_err(|_| Error::new(ErrorKind::Other, "dumping to file"));

        let (reader, writer) = stream.split();
        let (tx, rx) = futures::sync::mpsc::unbounded::<Vec<u8>>();
        let tx0 = tx.clone();
        let tx1 = tx.clone();

        let (ctx, crx) = futures::sync::mpsc::unbounded::<Vec<u8>>();
        let null_reader = crx.fold(0, |_, _| {
            lazy(|| ok::<u32, u32>(1))
            .map(|_| 0)
            .map_err(|_| ())
        })
        .map_err(|_| Error::new(ErrorKind::Other, "writing to null"));

        let timer = Timer::default();
        let ping =
        
        timer.interval(Duration::from_secs(5))
        .fold(tx0, move |tx, _| {

            let ping = mumble::Ping::new();
            let s = ping.compute_size();
            let mut buf = vec![0u8; (s + 6) as usize];
            buf[1] = 3u8; // Packet type: Ping
            (&mut buf[2..]).write_u32::<BigEndian>(s).unwrap();
            {
            let os = &mut CodedOutputStream::bytes(&mut buf[6..]);
            assert!(os.flush().is_ok());
            assert!(ping.write_to_with_cached_sizes(os).is_ok());
            }

            println!("PING");

            tx.send(buf)
            .map_err(|_| tokio_timer::TimerError::NoCapacity)
        })
        .map_err(|e| Error::new(ErrorKind::Other, e.to_string()));

        let socket_reader = loop_fn((reader, false, tx1, file_tx, ctx), |(reader, _/*done*/, tx, file_tx, ctx)| {
            
            io::read_exact(reader, [0u8; 2])
            .and_then(|(reader, buf)| {
                let mut rdr = Cursor::new(buf);
                let mum_type = rdr.read_u16::<BigEndian>().unwrap();
                println!("mum_type: {:?} {:?}", mum_type, buf);
                io::read_exact(reader, [0u8; 4])
                .and_then(move |(reader, buf)| {
                    ok((reader, buf, mum_type))
                })
            })
            .and_then(|(reader, buf, mum_type)| {
                let mut rdr = Cursor::new(buf);
                let mum_length = rdr.read_u32::<BigEndian>().unwrap();
                io::read_exact(reader, vec![0u8; mum_length as usize])
                .and_then(move |(reader, buf)| {
                    ok((reader, buf, mum_type))
                })
            })
            .and_then(move |(reader, buf, mum_type)| {

                match mum_type {
                    1 => { // UDPTunnel
                        // println!("saving to file");
                        
                        let mut rdr = Cursor::new(buf);
                        let aud_header = rdr.read_u8().unwrap();
                        let aud_type = aud_header & 0b11100000;
                        let aud_target = aud_header & 0b00011111;
                        println!("type: {} target: {}", aud_type, aud_target);

                        match aud_type {
                            128 => { // OPUS encoded voice data                                
                                let aud_session = rdr.read_varint().unwrap();
                                let aud_sequence = rdr.read_varint().unwrap();
                                println!("session: {} sequence: {}", aud_session, aud_sequence);
                                
                                let opus_length = rdr.read_varint().unwrap();
                                let opus_done = if opus_length & 0x2000 == 0x2000 {true} else {false};
                                let opus_length = opus_length & 0x1FFF;
                                println!("opus length: {} done: {}", opus_length, opus_done);
                                
                                let mut opus_data = vec![0u8; opus_length as usize];
                                rdr.read_exact(&mut opus_data[..]).unwrap();
                                
                                let file_tx0 = file_tx.clone();
                                file_tx.send(opus_data)
                                .map_err(|e| Error::new(ErrorKind::Other, e.to_string()))
                                .and_then(|_| ok((reader, false, tx, file_tx0, ctx)))
                                .boxed()
                            },
                            _ => {
                                let ctx0 = ctx.clone();
                                ctx.send(rdr.into_inner())
                                .map_err(|e| Error::new(ErrorKind::Other, e.to_string()))
                                .and_then(|_| ok((reader, false, tx, file_tx, ctx0)))
                                .boxed()
                            },
                        }
                    },
                    _ => {
                        let ctx0 = ctx.clone();
                        ctx.send(buf)
                        .map_err(|e| Error::new(ErrorKind::Other, e.to_string()))
                        .and_then(|_| ok((reader, false, tx, file_tx, ctx0)))
                        .boxed()
                    },
                }
            })
            .and_then(|(reader, done, tx, file_tx, ctx)| {
                if done {
                    Ok(Loop::Break(900))
                }
                else {
                    Ok(Loop::Continue((reader, false, tx, file_tx, ctx)))
                }
            })
        });

        let socket_writer = rx.fold(writer, move |writer, msg : Vec<u8>| {
            println!("{:?}", msg);
            io::write_all(writer, msg)
            .map(|(writer, _)| writer)
            .map_err(|_| ())
        })
        .map_err(|_| Error::new(ErrorKind::Other, "writing to tcp"));

        Future::join5(ping, socket_reader, null_reader, socket_writer, file_writer)
    });

    core.run(data).unwrap();
}