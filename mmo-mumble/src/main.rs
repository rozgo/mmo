extern crate clap;
extern crate openssl;
extern crate tokio_core;
extern crate tokio_openssl;
extern crate tokio_io;
extern crate tokio_file_unix;
extern crate tokio_timer;
extern crate protobuf;
extern crate byteorder;
extern crate opus;
extern crate chrono;
extern crate hyper;
extern crate hyper_tls;
extern crate rand;
extern crate pretty_env_logger;

extern crate warheadhateus;

use clap::{Arg, App};extern crate futures;

use std::fs;

use std::io::{Write, Read, Error, ErrorKind};
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use futures::Sink;
use futures::future::{Future, ok, loop_fn, Loop};
use futures::stream::{Stream};

use openssl::ssl::{SslContext, SslMethod, SSL_VERIFY_PEER};
use openssl::x509::X509_FILETYPE_PEM;

use tokio_io::AsyncRead;
use tokio_core::net::TcpStream;
use tokio_core::reactor::Core;
use tokio_timer::Timer;

use protobuf::Message;
use protobuf::{CodedOutputStream, CodedInputStream};

use std::io::Cursor;
use byteorder::{BigEndian, LittleEndian, ReadBytesExt, WriteBytesExt};

mod mumble;
mod connector;
use connector::MumbleConnector;

mod varint;
use varint::VarintReader;

mod lex;

fn err_str<T>(e : T) -> Error
where T: std::string::ToString
{
    Error::new(ErrorKind::Other, e.to_string())
}

fn app() -> App<'static, 'static> {
    App::new("mmo-mumble")
        .version("0.1.0")
        .about("Voice client bot!")
        .author("Alex Rozgo")
        .arg(Arg::with_name("addr").short("a").long("address").help("Host to connect to address:port").takes_value(true))
}

fn main() {

    let matches = app().get_matches();

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

    }).and_then(|stream| { // Version
        let mut version = mumble::Version::new();
        version.set_version(66052);
        version.set_release("1.2.4-0.2ubuntu1.1".to_string());
        version.set_os("X11".to_string());
        version.set_os_version("Ubuntu 14.04.5 LTS".to_string());
        let s = version.compute_size();
        println!("version size: {}", s);
        let mut buf = vec![0u8; (s + 6) as usize];
        (&mut buf[0..]).write_u16::<BigEndian>(0).unwrap(); // Packet type: Version
        (&mut buf[2..]).write_u32::<BigEndian>(s).unwrap();
        {
        let os = &mut CodedOutputStream::bytes(&mut buf[6..]);
        assert!(os.flush().is_ok());
        assert!(version.write_to_with_cached_sizes(os).is_ok());
        }
        tokio_io::io::write_all(stream, buf)

    }).and_then(|(stream, _)| { // Authenticate
        let mut auth = mumble::Authenticate::new();
        auth.set_username("lyric".to_string());
        auth.set_opus(true);
        let s = auth.compute_size();
        println!("auth size: {}", s);
        let mut buf = vec![0u8; (s + 6) as usize];
        (&mut buf[0..]).write_u16::<BigEndian>(2).unwrap(); // Packet type: Authenticate
        (&mut buf[2..]).write_u32::<BigEndian>(s).unwrap();
        {
        let os = &mut CodedOutputStream::bytes(&mut buf[6..]);
        assert!(os.flush().is_ok());
        assert!(auth.write_to_with_cached_sizes(os).is_ok());
        }
        tokio_io::io::write_all(stream, buf)

    }).and_then(|(stream, _)| {

        let (reader, writer) = stream.split();
        let (tx, rx) = futures::sync::mpsc::unbounded::<Vec<u8>>();
        let tx0 = tx.clone();
        let tx1 = tx.clone();

        let socket_writer = rx.fold(writer, move |writer, msg : Vec<u8>| {
            println!("{:?}", msg);
            tokio_io::io::write_all(writer, msg)
            .map(|(writer, _)| writer)
            .map_err(|_| ())
        })
        .map_err(|_| Error::new(ErrorKind::Other, "writing to tcp"));

        let lex_client = lex::client(&handle);
        let (lex_tx, lex_rx) = futures::sync::mpsc::unbounded::<Vec<u8>>();
        let lex_task = lex_rx.fold((), move |(), msg| {
            let mut req = lex::request();
            req.set_body(msg);
            lex_client.request(req).and_then(|res| {
                println!("POST: {:?}", res.headers());
                res.body().for_each(|chunk| {
                    // file.write_all(&chunk).map_err(From::from)
                    // std::io::stdout().write_all(&chunk).map_err(From::from)
                    std::io::stdout().write_all(b"").map_err(From::from)
                })
            })
            .map(|_| ())
            .map_err(|_| ())
        })
        .map_err(|_| Error::new(ErrorKind::Other, "dumping to file"));

        let (file_tx, file_rx) = futures::sync::mpsc::unbounded::<(Vec<u8>, bool)>();
        let file : Option<_> = None;
        let file_writer = file_rx.fold(file, move |writer, (msg, done)| {
            let mut writer = 
                match writer {
                    Some (writer) => {
                        writer
                    },
                    None => {
                        // let date = chrono::Local::now().format("%Y-%m-%d-%H-%M-%S");
                        // let file_name = format!("dump-{}.opus", date);
                        // let file = fs::File::create(file_name).unwrap();
                        // tokio_file_unix::File::new_nb(file).unwrap()
                        let opus_data = Vec::<u8>::new();
                        let opus_data = Cursor::new(opus_data);
                        opus_data
                    }
                };
            println!("dumping {:?}", msg.len());
            writer.write_all(&msg).unwrap();

            if done {
                let data = writer.into_inner();
                {
                    let date = chrono::Local::now().format("%Y-%m-%d-%H-%M-%S");
                    let file_name = format!("dump-{}.opus", date);
                    let mut file = fs::File::create(file_name).unwrap();
                    file.write_all(&data).unwrap();
                }
                let lex_tx = lex_tx.clone();
                lex_tx.send(data)
                .map_err(|e| Error::new(ErrorKind::Other, e.to_string()))
                .and_then(|_| ok(None))
                .boxed()
                }
            else {
                ok(Some(writer))
                .boxed()
            }
            .map_err(|_| ())
        })
        .map(|_| ())
        .map_err(|_| Error::new(ErrorKind::Other, "dumping to file"));

        let timer = Timer::default();
        let ping = timer.interval(Duration::from_secs(5)).fold(tx0, move |tx, _| {
            let ping = mumble::Ping::new();
            let s = ping.compute_size();
            let mut buf = vec![0u8; (s + 6) as usize];
            (&mut buf[0..]).write_u16::<BigEndian>(3).unwrap(); // Packet type: Ping
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

        let socket_reader = loop_fn((reader, tx1, file_tx), |(reader, tx, file_tx)| {
            
            tokio_io::io::read_exact(reader, [0u8; 2])
            .and_then(|(reader, buf)| {
                let mut rdr = Cursor::new(&buf);
                let mum_type = rdr.read_u16::<BigEndian>().unwrap();
                println!("mum_type: {}", mum_type);
                tokio_io::io::read_exact(reader, [0u8; 4])
                .and_then(move |(reader, buf)| {
                    ok((reader, buf, mum_type))
                })
            })
            .and_then(|(reader, buf, mum_type)| {
                let mut rdr = Cursor::new(&buf);
                let mum_length = rdr.read_u32::<BigEndian>().unwrap();
                tokio_io::io::read_exact(reader, vec![0u8; mum_length as usize])
                .and_then(move |(reader, buf)| {
                    ok((reader, buf, mum_type))
                })
            })
            .and_then(move |(reader, buf, mum_type)| {

                match mum_type {
                    0 => { // Version
                        let mut inp = CodedInputStream::from_bytes(&buf);
                        let mut msg = mumble::Version::new();
                        msg.merge_from(&mut inp).unwrap();
                        println!("Version: {:?}", msg);
                        ok((reader, tx, file_tx))
                        .boxed()
                    },
                    1 => { // UDPTunnel
                        println!("full load: {}", buf.len());
                        
                        let mut rdr = Cursor::new(&buf);
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
                                
                                let mut opus_data = vec![0xff; opus_length as usize];
                                rdr.read_exact(&mut opus_data[..]).unwrap();

                                let mut sample_pcm = vec![0i16; 500000];
                                let mut decoder = opus::Decoder::new(16000, opus::Channels::Mono).unwrap();
                                let size = decoder.decode(&opus_data[..], &mut sample_pcm[..], false).unwrap();
                                let mut opus_data = vec![0u8; size * 2];
                                for s in 0..size {
                                    (&mut opus_data[s*2..]).write_i16::<LittleEndian>(sample_pcm[s]).unwrap();
                                }
                                
                                let file_tx0 = file_tx.clone();
                                file_tx.send((opus_data, opus_done))
                                .map_err(|e| Error::new(ErrorKind::Other, e.to_string()))
                                .and_then(|_| ok((reader, tx, file_tx0)))
                                .boxed()
                            },
                            32 => { // Ping
                                ok((reader, tx, file_tx))
                                .boxed()
                            },
                            _ => {
                                panic!("aud_type");
                            }
                        }
                    },
                    5 => { // ServerSync
                        let mut inp = CodedInputStream::from_bytes(&buf);
                        let mut msg = mumble::ServerSync::new();
                        msg.merge_from(&mut inp).unwrap();
                        println!("ServerSync: {:?}", msg);
                        ok((reader, tx, file_tx))
                        .boxed()
                    },
                    7 => { // ChannelState
                        let mut inp = CodedInputStream::from_bytes(&buf);
                        let mut msg = mumble::ChannelState::new();
                        msg.merge_from(&mut inp).unwrap();
                        println!("ChannelState: {:?}", msg);
                        ok((reader, tx, file_tx))
                        .boxed()
                    },
                    9 => { // UserState
                        let mut inp = CodedInputStream::from_bytes(&buf);
                        let mut msg = mumble::UserState::new();
                        msg.merge_from(&mut inp).unwrap();
                        println!("UserState: {:?}", msg);
                        ok((reader, tx, file_tx))
                        .boxed()
                    },
                    11 => { // TextMessage
                        let mut inp = CodedInputStream::from_bytes(&buf);
                        let mut msg = mumble::TextMessage::new();
                        msg.merge_from(&mut inp).unwrap();
                        println!("TextMessage: {:?}", msg);
                        ok((reader, tx, file_tx))
                        .boxed()
                    },
                    15 => { // CryptSetup
                        let mut inp = CodedInputStream::from_bytes(&buf);
                        let mut msg = mumble::CryptSetup::new();
                        msg.merge_from(&mut inp).unwrap();
                        println!("CryptSetup: {:?}", msg);
                        ok((reader, tx, file_tx))
                        .boxed()
                    },
                    20 => { // PermissionQuery
                        let mut inp = CodedInputStream::from_bytes(&buf);
                        let mut msg = mumble::PermissionQuery::new();
                        msg.merge_from(&mut inp).unwrap();
                        println!("PermissionQuery: {:?}", msg);
                        ok((reader, tx, file_tx))
                        .boxed()
                    },
                    21 => { // CodecVersion
                        let mut inp = CodedInputStream::from_bytes(&buf);
                        let mut msg = mumble::CodecVersion::new();
                        msg.merge_from(&mut inp).unwrap();
                        println!("CodecVersion: {:?}", msg);
                        ok((reader, tx, file_tx))
                        .boxed()
                    },
                    24 => { // ServerConfig
                        let mut inp = CodedInputStream::from_bytes(&buf);
                        let mut msg = mumble::ServerConfig::new();
                        msg.merge_from(&mut inp).unwrap();
                        println!("ServerConfig: {:?}", msg);
                        ok((reader, tx, file_tx))
                        .boxed()
                    },
                    _ => {
                        ok((reader, tx, file_tx))
                        .boxed()
                    },
                }
            })
            .and_then(|(reader, tx, file_tx)| {
                if false {
                    Ok(Loop::Break(900))
                }
                else {
                    Ok(Loop::Continue((reader, tx, file_tx)))
                }
            })
        });
        
        Future::join5(ping, socket_reader, socket_writer, file_writer, lex_task)
    });

    core.run(data).unwrap();
}
