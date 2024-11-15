use std::io;
use std::time::SystemTime;
use std::io::{stdout, Read, Write};
use crate::{OK, ERROR, GzipState, STORED, DEFLATED, GZIP_MAGIC, ORIG_NAME, OS_CODE, INBUFSIZ, INBUF_EXTRA, OUTBUFSIZ, OUTBUF_EXTRA, DIST_BUFSIZE, WSIZE};
use crate::deflate::Deflate;
use crate::inflate::Inflate;
use crate::trees::Trees;

// Macros for getting two-byte and four-byte header values
/// 提取两字节无符号整数
fn SH(p: &[u8]) -> u16 {
    (p[0] as u16) | ((p[1] as u16) << 8)
}

/// 提取四字节无符号整数
fn LG(p: &[u8]) -> u32 {
    (SH(&p[0..2]) as u32) | ((SH(&p[2..4]) as u32) << 16)
}


/* PKZIP header definitions */
const LOCSIG: u32 = 0x04034b50; // four-byte lead-in (lsb first)
const LOCFLG: usize = 6;        // offset of bit flag
const CRPFLG: u32 = 1;          // bit for encrypted entry
const EXTFLG: u32 = 8;          // bit for extended local header
const LOCHOW: usize = 8;        // offset of compression method
// const LOCTIM: usize = 10;    // UNUSED file mod time (for decryption)
const LOCCRC: usize = 14;       // offset of crc
const LOCSIZ: usize = 18;       // offset of compressed size
const LOCLEN: usize = 22;       // offset of uncompressed length
const LOCFIL: usize = 26;       // offset of file name field length
const LOCEXT: usize = 28;       // offset of extra field length
const LOCHDR: usize = 30;       // size of local header, including sig
const EXTHDR: usize = 16;       // size of extended local header, inc sig
const RAND_HEAD_LEN: u32 = 12; // length of encryption random header

/* Globals */


pub fn unzip (state: &mut GzipState) -> io::Result<()> {
    let mut decrypt: i32 = 0;        // flag to turn on decryption
    let mut pkzip: i32 = 0;          // set for a pkzip file
    let mut ext_header: i32 = 0;     // set if extended local header
    let mut orig_crc: u32 = 0;        // original crc
    let mut orig_len: u32 = 0;        // original uncompressed length
    let mut n: i32;
    let mut buf: [u8; EXTHDR] = [0; EXTHDR]; // extended local header
    let mut err = OK;
//     let mut inbuf: [u8; INBUFSIZ + INBUF_EXTRA] = [0; INBUFSIZ + INBUF_EXTRA];
//     let mut outbuf: [u8; OUTBUFSIZ + OUTBUF_EXTRA] = [0; OUTBUFSIZ + OUTBUF_EXTRA];
//     let mut d_buf: [u8; DIST_BUFSIZE] = [0; DIST_BUFSIZE];
//     let mut window: [u8; 2 * WSIZE] = [0; 2 * WSIZE];

    let mut inflate = Inflate::new();

    state.updcrc(None, 0); // initialize crc

    if pkzip>0 && ext_header == 0 {  // crc and length at the end otherwise
        orig_crc = LG(&state.inbuf[LOCCRC..]);
        orig_len = LG(&state.inbuf[LOCLEN..]);
    }

    // Decompress
    if state.method == DEFLATED {
        let res = inflate.inflate(state);

        if res == 3 {
            state.gzip_error("memory exhausted");
        } else if res != 0 {
            state.gzip_error("invalid compressed data--format violated");
        }
    } else if pkzip>0 && state.method == STORED {
        let mut n = LG(&state.inbuf[LOCLEN..]);

        if n != LG(&state.inbuf[LOCSIZ..]) - (decrypt != 0) as u32 * RAND_HEAD_LEN {
            eprintln!("len {}, siz {}", n, LG(&state.inbuf[LOCSIZ..]));
            state.gzip_error("invalid compressed data--length mismatch");
        }
        while n > 0 {
            let c: u8 = inflate.get_byte(state)?;
            state.put_byte(c);
            n -= 1;
        }
        inflate.flush_window(state);
    } else {
        state.gzip_error("internal error, invalid method");
    }

    // Get the crc and original length
    if pkzip == 0 {
        // crc32 (see algorithm.doc)
        // uncompressed input size modulo 2^32
        for n in 0..8 {
            buf[n] = inflate.get_byte(state)?; // may cause an error if EOF
        }
        orig_crc = LG(&buf);
        orig_len = LG(&buf[4..]);
    } else if ext_header>0 {
        // If extended header, check it
        // signature - 4bytes: 0x50 0x4b 0x07 0x08
        // CRC-32 value
        // compressed size 4-bytes
        // uncompressed size 4-bytes
        for n in 0..EXTHDR {
            buf[n] = inflate.get_byte(state)?; // may cause an error if EOF
        }
        orig_crc = LG(&buf[4..]);
        orig_len = LG(&buf[12..]);
    }


    // Validate decompression
    if  u32::from(orig_crc) != state.updcrc(Some(&state.outbuf.clone()), 0) {
        eprintln!(
            "\n{}: {}: invalid compressed data--crc error",
            state.program_name, state.ifname
        );
        err = ERROR;
    }
    if  u32::from(orig_len) != (state.bytes_out & 0xffffffff) as u32 {
        eprintln!(
            "\n{}: {}: invalid compressed data--length error",
            state.program_name, state.ifname
        );
        err = ERROR;
    }

    // Check if there are more entries in a pkzip file
    if pkzip>0 && state.inptr + 4 < state.insize && LG(&state.inbuf[state.inptr..] ) == LOCSIG {
        if state.to_stdout {
            eprintln!(
                "{}: {} has more than one entry--rest ignored",
                state.program_name, state.ifname
            );
        } else {
            // Don't destroy the input zip file
            eprintln!(
                "{}: {} has more than one entry -- unchanged",
                state.program_name, state.ifname
            );
            err = ERROR;
        }
    }
    ext_header = 0; // for next file
    pkzip = 0;

    if err == OK {
        return Ok(());
    }

    state.exit_code = ERROR;
//     if !test {
//         abort_gzip();
//     }

    return Err(io::Error::new(io::ErrorKind::Other, "Decompression error"));
}
