use std::io::{self, Result};

/// let mut rdr = Cursor::new(buf);
/// let num = rdr.read_varint().unwrap();
pub trait VarintReader: io::Read {

    fn read_varint(&mut self) -> Result<u64> {

        fn decode<F>(i : u64, mut next : F) -> u64
        where F : FnMut() -> u64 {
            let mut i = i;
            let v = next();

            if v & 0x80 == 0x00 {
                i = v & 0x7F
            }
            else if v & 0xC0 == 0x80 {
                i = (v & 0x3F) << 8 | next()
            }
            else if (v & 0xF0) == 0xF0 {
                i = match v & 0xFC {
                    0xF0 => next() << 24 | next() << 16 | next() << 8 | next(),
                    0xF4 => next() << 56 | next() << 48 | next() << 40 | next() << 32 | next() << 24 | next() << 16 | next() << 8 | next(),
                    0xF8 => {
                        panic!("read_varint recursion");
                        i = decode(i, next);
                        !i
                    },
                    0xFC => {
                        i = v & 0x03;
                        !i
                    },
                    _ => panic!("what"),
                }
            }
            else if (v & 0xF0) == 0xE0 {
                i = (v & 0x0F) << 24 | next() << 16 | next() << 8 | next()
            }
            else if (v & 0xE0) == 0xC0 {
                i = (v & 0x1F) << 16 | next() << 8 | next()
            }

            i
        }
            
        let next = || -> u64 {
            let mut buf = [0u8; 1];
            self.read_exact(&mut buf).unwrap();
            buf[0] as u64
        };

        let mut i = 0u64;
    
        i = decode(i, next);

        Ok(i)
    }
}

impl<R: io::Read + ?Sized> VarintReader for R {}
