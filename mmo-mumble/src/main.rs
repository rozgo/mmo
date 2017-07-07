#![feature(rustc_private)]

extern crate pretty_env_logger;
#[macro_use] extern crate log;

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

extern crate warheadhateus;

extern crate serde;
extern crate toml;
#[macro_use]
extern crate serde_derive;

use clap::{Arg, App};extern crate futures;

use std::fs;

use std::io::Cursor;
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

use protobuf::Message;
use protobuf::{CodedOutputStream, CodedInputStream};

use byteorder::{BigEndian, LittleEndian, ReadBytesExt, WriteBytesExt};

mod mumble;
mod connector;
use connector::MumbleConnector;

mod varint;
use varint::VarintReader;
use varint::VarintWriter;

mod lex;

mod rnd;

mod config;

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
        .arg(Arg::with_name("cfg").short("c").long("config").help("Path to config toml").takes_value(true))
}

fn main() {
    pretty_env_logger::init().unwrap();

    let matches = app().get_matches();

    let config : config::Config = {
        let config_file = matches.value_of("cfg").unwrap_or("Config.toml");
        let mut config_file = fs::File::open(config_file).unwrap();
        let mut config = String::new();
        config_file.read_to_string(&mut config).unwrap();
        toml::from_str(&config).unwrap()
    };

    let access_key_id = &config.aws.access_key_id;
    let secret_access_key = &config.aws.secret_access_key;
    println!("access_key_id {}", access_key_id);
    println!("secret_access_key {}", secret_access_key);

    let addr_str = matches.value_of("addr").unwrap_or("127.0.0.1:8080");
    let addr = addr_str.parse::<SocketAddr>().unwrap();

    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let client = TcpStream::connect(&addr, &handle);
    
    let app_logic = client.and_then(|socket| {
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

        let (mum_reader, mum_writer) = stream.split();

        // mumble writer
        let (mum_tx, mum_rx) = futures::sync::mpsc::unbounded::<Vec<u8>>();
        let mum_writer = mum_rx.fold(mum_writer, move |writer, msg : Vec<u8>| {
            if msg.len() < 10 {
                // println!("PING");
                println!("MSG {:?}", msg);
            }
            // println!("MSG {:?}", msg);
            tokio_io::io::write_all(writer, msg)
            .map(|(writer, _)| writer)
            .map_err(|_| ())
        })
        .map_err(|_| Error::new(ErrorKind::Other, "writing to tcp"));

        // lex request
        let (lex_tx, lex_rx) = futures::sync::mpsc::unbounded::<(Vec<u8>, u32)>();
        let lex_task = lex_rx.fold(mum_tx.clone(), |mum_tx, (msg, session)| {

            let mut req = lex::request(&config, session);
            req.set_body(msg);
            println!("req session: {}", session);

            let mum_tx_next = mum_tx.clone();
            let lex_client = lex::client(&handle);
            lex_client.request(req).and_then(move |res| {

                // let mum_lex_tx = mum_lex_tx.clone();

                println!("req POST: {:?}", res.headers());
                let vox_data = Vec::<u8>::new();
                let vox_data = Cursor::<Vec<u8>>::new(vox_data);
                res.body().fold(vox_data, |mut vox_data, chunk| {
                    println!("chuck: {}", chunk.len());
                    vox_data.write_all(&chunk).unwrap();                    
                    ok::<Cursor<Vec<u8>>, hyper::Error>(vox_data)
                })
                .and_then(move |vox_data| {

                    let mum_tx = mum_tx.clone();

                    // raw audio
                    let vox_data = vox_data.into_inner();
                    {
                        let date = chrono::Local::now().format("%Y-%m-%d-%H-%M-%S");
                        let file_name = format!("outgoing-{}.opus", date);
                        let mut file = fs::File::create(file_name).unwrap();
                        file.write_all(&vox_data).unwrap();
                    }

                    // pcm audio
                    let mut vox_pcm = Vec::<i16>::new();
                    {
                        let mut cur = Cursor::new(&vox_data);
                        while let Ok(i) = cur.read_i16::<LittleEndian>() {
                            vox_pcm.push(i);
                        }
                    }

                    println!("BODY DONE, size: {}", vox_data.len());

                    // opus frames
                    let mut encoder = opus::Encoder::new(16000, opus::Channels::Mono, opus::Application::Voip).unwrap();
                    let mut opus = Vec::<Result<(bool,Vec<u8>),opus::Error>>::new();
                    let chunks = vox_pcm.chunks(320);
                    for chunk in chunks {
                        let mut frame = vec![0i16; 320];
                        for (i, v) in chunk.iter().enumerate() {
                            frame[i] = *v;
                        }
                        let frame = encoder.encode_vec(&frame, 4000).unwrap();
                        // util::opus_analyze(&frame);
                        opus.push(Ok((false, frame)));
                    }

                    println!("opus count: {}", opus.len());

                    // opus_analyze(&frames);

                    if let Some(Ok((_, frame))) = opus.pop() {
                        opus.push(Ok((true, frame)));
                    }

                    let mut p = (chrono::UTC::now().timestamp() as u64) & 0x000000000000FFFFu64;
                    // p = 0;

                    futures::stream::iter(opus).for_each(move |opus| {
                        let (done, opus) = opus;
                        p = p + 1;
                        let aud_header = 0b100 << 5;
                        // println!("outgoing aud_header: {}", aud_header);
                        let data = Vec::<u8>::new();
                        let mut data = Cursor::new(data);
                        data.write_u8(aud_header).unwrap();
                        data.write_varint(p).unwrap();
                        let opus_len =
                            if done { 
                                println!("OPUS END: prev:{} next:{}", opus.len(), opus.len() as u64 | 0x2000);
                                opus.len() as u64 | 0x2000 }
                            else { opus.len() as u64 };
                        data.write_varint(opus_len).unwrap();
                        // opus_analyze(&opus);
                        data.write_all(&opus).unwrap();
                        let data = data.into_inner();
                        println!("p: {} opus: {} data: {}", p, opus.len(), data.len());
                        let mum_tx = mum_tx.clone();
                        mum_tx.send(data)
                        .map(|_| ())
                        .map_err(|_| opus::Error::new("who knows", opus::ErrorCode::BadArg))
                    })
                    .map_err(|_| hyper::Error::Header)

                    // futures::stream::iter(opus).for_each(move |opus| {

                    //     let mut msg = mumble::TextMessage::new();
                    //     msg.set_actor(session);
                    //     msg.set_channel_id(vec![0]);
                    //     msg.set_message("just wanna say supercalifragilisticexpialidocious".to_string());
                    //     let s = msg.compute_size();
                    //     let mut buf = vec![0u8; (s + 6) as usize];
                    //     (&mut buf[0..]).write_u16::<BigEndian>(11).unwrap(); // Packet type: TextMessage
                    //     (&mut buf[2..]).write_u32::<BigEndian>(s).unwrap();
                    //     {
                    //     let os = &mut CodedOutputStream::bytes(&mut buf[6..]);
                    //     assert!(os.flush().is_ok());
                    //     assert!(msg.write_to_with_cached_sizes(os).is_ok());
                    //     }

                    //     let mum_tx = mum_tx.clone();
                    //     mum_tx.send(buf)
                    //     .map(|_| ())
                    //     .map_err(|_| opus::Error::new("who knows", opus::ErrorCode::BadArg))
                    // })
                    // .map_err(|_| hyper::Error::Header)

                    // let mut msg = mumble::TextMessage::new();
                    // msg.set_actor(session);
                    // msg.set_channel_id(vec![0]);
                    // msg.set_message("just wanna say supercalifragilisticexpialidocious".to_string());
                    // let s = msg.compute_size();
                    // let mut buf = vec![0u8; (s + 6) as usize];
                    // (&mut buf[0..]).write_u16::<BigEndian>(11).unwrap(); // Packet type: TextMessage
                    // (&mut buf[2..]).write_u32::<BigEndian>(s).unwrap();
                    // {
                    // let os = &mut CodedOutputStream::bytes(&mut buf[6..]);
                    // assert!(os.flush().is_ok());
                    // assert!(msg.write_to_with_cached_sizes(os).is_ok());
                    // }
                    // mum_tx.send(buf)
                    // .map_err(|_| hyper::Error::Header)
                })
            })
            .map(|_| mum_tx_next)
            .map_err(|_| ())
        })
        .map_err(|_| Error::new(ErrorKind::Other, "lex request"));

        // voice buffer
        let (vox_tx, vox_rx) = futures::sync::mpsc::unbounded::<(Vec<u8>, u32, bool)>();
        let vox_buf : Option<_> = None;
        let vox_writer = vox_rx.fold(vox_buf, move |writer, (msg, session, done)| {
            let mut writer = 
                match writer {
                    Some (writer) => writer,
                    None => Cursor::new(Vec::<u8>::new()),
                };
            trace!("vox_chunk {:?}", msg.len());
            writer.write_all(&msg).unwrap();
            if done {
                let vox_data = writer.into_inner();
                {
                    let date = chrono::Local::now().format("%Y-%m-%d-%H-%M-%S");
                    let file_name = format!("incoming-{}.opus", date);
                    let mut file = fs::File::create(file_name).unwrap();
                    file.write_all(&vox_data).unwrap();
                }
                let lex_tx = lex_tx.clone();
                lex_tx.send((vox_data, session))
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

        // mumble ping
        let timer = tokio_timer::Timer::default();
        let ping = timer.interval(Duration::from_secs(5)).fold(mum_tx.clone(), move |tx, _| {
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
            
            tx.send(buf)
            .map_err(|_| tokio_timer::TimerError::NoCapacity)
        })
        .map_err(|e| Error::new(ErrorKind::Other, e.to_string()));

        // mumble reader
        let mum_reader = loop_fn((mum_reader, mum_tx.clone(), vox_tx, 0u32), |(mum_reader, mum_tx, vox_tx, session)| {
            
            // packet type
            tokio_io::io::read_exact(mum_reader, [0u8; 2])
            .and_then(|(mum_reader, buf)| {
                let mut rdr = Cursor::new(&buf);
                let mum_type = rdr.read_u16::<BigEndian>().unwrap();
                trace!("mum_type: {}", mum_type);
                tokio_io::io::read_exact(mum_reader, [0u8; 4])
                .and_then(move |(mum_reader, buf)| {
                    ok((mum_reader, buf, mum_type))
                })
            })

            // packet length
            .and_then(|(mum_reader, buf, mum_type)| {
                let mut rdr = Cursor::new(&buf);
                let mum_length = rdr.read_u32::<BigEndian>().unwrap();
                tokio_io::io::read_exact(mum_reader, vec![0u8; mum_length as usize])
                .and_then(move |(mum_reader, buf)| {
                    ok((mum_reader, buf, mum_type))
                })
            })

            // packet payload
            .and_then(move |(mum_reader, buf, mum_type)| {

                match mum_type {
                    0 => { // Version
                        let mut inp = CodedInputStream::from_bytes(&buf);
                        let mut msg = mumble::Version::new();
                        msg.merge_from(&mut inp).unwrap();
                        debug!("version: {:?}", msg);
                        ok((mum_reader, mum_tx, vox_tx, session))
                        .boxed()
                    },
                    1 => { // UDPTunnel
                        trace!("full load: {}", buf.len());
                        
                        let mut rdr = Cursor::new(&buf);
                        let aud_header = rdr.read_u8().unwrap();
                        // println!("incoming aud_header: {}", aud_header);
                        let aud_type = aud_header & 0b11100000;
                        let aud_target = aud_header & 0b00011111;
                        debug!("type: {} target: {}", aud_type, aud_target);

                        match aud_type {
                            128 => { // OPUS encoded voice data                                
                                // let aud_session = rdr.read_varint().unwrap();
                                // let aud_sequence = rdr.read_varint().unwrap();
                                // println!("session: {} sequence: {}", aud_session, aud_sequence);

                                let opus_length = rdr.read_varint().unwrap();
                                let opus_done = if opus_length & 0x2000 == 0x2000 {true} else {false};
                                let opus_length = opus_length & 0x1FFF;
                                // println!("opus length: {} done: {}", opus_length, opus_done);
                                
                                let mut opus_data = vec![0u8; opus_length as usize];
                                rdr.read_exact(&mut opus_data[..]).unwrap();

                                // opus_analyze(&opus_data);

                                let mut sample_pcm = vec![0i16; 320];
                                let mut decoder = opus::Decoder::new(16000, opus::Channels::Mono).unwrap();
                                let size = decoder.decode(&opus_data[..], &mut sample_pcm[..], false).unwrap();
                                let mut opus_data = vec![0u8; size * 2];
                                for s in 0..size {
                                    (&mut opus_data[s*2..s*2+2]).write_i16::<LittleEndian>(sample_pcm[s]).unwrap();
                                }
                                
                                let vox_tx0 = vox_tx.clone();
                                vox_tx.send((opus_data, session, opus_done))
                                .map_err(|e| Error::new(ErrorKind::Other, e.to_string()))
                                .and_then(move |_| ok((mum_reader, mum_tx, vox_tx0, session)))
                                .boxed()
                            },
                            32 => { // Ping
                                ok((mum_reader, mum_tx, vox_tx, session))
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
                        ok((mum_reader, mum_tx, vox_tx, msg.get_session()))
                        .boxed()
                    },
                    7 => { // ChannelState
                        let mut inp = CodedInputStream::from_bytes(&buf);
                        let mut msg = mumble::ChannelState::new();
                        msg.merge_from(&mut inp).unwrap();
                        println!("ChannelState: {:?}", msg);
                        ok((mum_reader, mum_tx, vox_tx, session))
                        .boxed()
                    },
                    9 => { // UserState
                        let mut inp = CodedInputStream::from_bytes(&buf);
                        let mut msg = mumble::UserState::new();
                        msg.merge_from(&mut inp).unwrap();
                        println!("UserState: {:?}", msg);
                        ok((mum_reader, mum_tx, vox_tx, session))
                        .boxed()
                    },
                    11 => { // TextMessage
                        let mut inp = CodedInputStream::from_bytes(&buf);
                        let mut msg = mumble::TextMessage::new();
                        msg.merge_from(&mut inp).unwrap();
                        println!("TextMessage: {:?}", msg);
                        ok((mum_reader, mum_tx, vox_tx, session))
                        .boxed()
                    },
                    15 => { // CryptSetup
                        let mut inp = CodedInputStream::from_bytes(&buf);
                        let mut msg = mumble::CryptSetup::new();
                        msg.merge_from(&mut inp).unwrap();
                        println!("CryptSetup: {:?}", msg);
                        ok((mum_reader, mum_tx, vox_tx, session))
                        .boxed()
                    },
                    20 => { // PermissionQuery
                        let mut inp = CodedInputStream::from_bytes(&buf);
                        let mut msg = mumble::PermissionQuery::new();
                        msg.merge_from(&mut inp).unwrap();
                        println!("PermissionQuery: {:?}", msg);
                        ok((mum_reader, mum_tx, vox_tx, session))
                        .boxed()
                    },
                    21 => { // CodecVersion
                        let mut inp = CodedInputStream::from_bytes(&buf);
                        let mut msg = mumble::CodecVersion::new();
                        msg.merge_from(&mut inp).unwrap();
                        println!("CodecVersion: {:?}", msg);
                        ok((mum_reader, mum_tx, vox_tx, session))
                        .boxed()
                    },
                    24 => { // ServerConfig
                        let mut inp = CodedInputStream::from_bytes(&buf);
                        let mut msg = mumble::ServerConfig::new();
                        msg.merge_from(&mut inp).unwrap();
                        println!("ServerConfig: {:?}", msg);
                        ok((mum_reader, mum_tx, vox_tx, session))
                        .boxed()
                    },
                    _ => {
                        ok((mum_reader, mum_tx, vox_tx, session))
                        .boxed()
                    },
                }
            })
            .and_then(|(mum_reader, mum_tx, vox_tx, session)| {
                if false {
                    Ok(Loop::Break(900))
                }
                else {
                    Ok(Loop::Continue((mum_reader, mum_tx, vox_tx, session)))
                }
            })
        });

        Future::join5(ping, mum_reader, mum_writer, vox_writer, lex_task)
    });

    core.run(app_logic).unwrap();
}
