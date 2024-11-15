use std::io;
use crate::GzipState;
use crate::trees::Trees;
use crate::{OK, ERROR, STORED, WSIZE, INBUFSIZ};
use std::io::{stdout, Read, Write};

#[derive(Debug)]
struct Huft {
    v: Box<Option<Box<Huft>>>, // Pointer to next level of table or value
    e: u8, // Extra bits for the current table
    b: u8, // Number of bits for this code or subcode
}

enum HuftValue {
    N(u16),         // literal, length base, or distance base
    T(Box<Huft>),   // pointer to next level of table
}

fn huft_free(t: Option<Box<Huft>>) -> i32 {
    let mut p = t;
    while let Some(mut q) = p {
        p = q.v.take();
    }
    0
}

// Order of the bit length code lengths
static border: [u16; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

// Copy lengths for literal codes 257..285
static cplens: [u16; 31] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31,
    35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258, 0,
    0,
];

// Extra bits for literal codes 257..285
static cplext: [u16; 31] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2,
    3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0, 99, 99,
]; // 99==invalid

// Copy offsets for distance codes 0..29
static cpdist: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193,
    257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145,
    8193, 12289, 16385, 24577,
];

// Extra bits for distance codes
static cpdext: [u16; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6,
    7, 7, 8, 8, 9, 9, 10, 10, 11, 11,
    12, 12, 13, 13,
];

// Mask bits array equivalent in Rust
static mask_bits: [u16; 17] = [
    0x0000,
    0x0001, 0x0003, 0x0007, 0x000f, 0x001f, 0x003f, 0x007f, 0x00ff,
    0x01ff, 0x03ff, 0x07ff, 0x0fff, 0x1fff, 0x3fff, 0x7fff, 0xffff,
];



// Constants
const BMAX: i32 = 16;      // maximum bit length of any code (16 for explode)
const N_MAX: i32 = 288;    // maximum number of codes in any set

// Function prototypes
// static mut HUFT_FREE: fn(*mut Huft) -> i32 = huft_free;

pub struct Inflate {
    bb: u64,
    bk: u32,
    wp: usize,
    lbits: i32,
    dbits: i32,
    hufts: u32
}

impl Inflate {
    pub fn new() -> Self {
        Self {
            bb: 0,
            bk: 0,
            wp: 0,
            lbits: 9,
            dbits: 6,
            hufts: 0
        }
    }

    pub fn slide(&mut self, state: &mut GzipState) -> &mut [u8; 2*WSIZE] {
        &mut state.window
    }

    pub fn fill_inbuf<R: Read>(&mut self, input: &mut R, eof_ok: bool, state: &mut GzipState) -> io::Result<u8> {
        state.insize = 0;
        loop {
            let len = self.read_buffer(input)?;
            if len == 0 {
                break;
            }
            state.insize += len;
            if state.insize >= INBUFSIZ {
                break;
            }
        }

        if state.insize == 0 {
            if eof_ok {
                return Ok(EOF as u8);
            }
            self.flush_window(state)?;
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "Unexpected EOF"));
        }
        state.bytes_in += state.insize as u64;
        state.inptr = 1;
        Ok(state.inbuf[0])
    }

    pub fn read_buffer<R: Read>(&mut self, input: &mut R, state: &mut GzipState) -> io::Result<usize> {
        let buffer = &mut state.inbuf[state.insize..INBUFSIZ];
        let len = input.read(buffer)?;

        // Output the data read and its length for debugging
        if len > 0 {
            print!("Read {} bytes: ", len);
            for byte in &buffer[..len] {
                print!("{:02x} ", byte);
            }
            println!();
        } else if len == 0 {
            println!("Reached end of file");
        }

        Ok(len)
    }

    pub fn flush_window(&mut self, state: &mut GzipState) {
        if state.outcnt == 0 {
            return;
        }

        state.updcrc(&state.window, state.outcnt);

        if !self.test {
            self.write_buf(&mut state.ofd, &state.window[0..state.outcnt], state.outcnt);
        }

        state.bytes_out += state.outcnt as i64;
        state.outcnt = 0;
    }

    // Function to flush output (equivalent to macro flush_output in C)
    pub fn flush_output(&mut self, state: &mut GzipState, w: usize) {
        unsafe {
            self.wp = w;
        }
        self.flush_window(state);
    }

    pub fn get_byte(&mut self, state: &mut GzipState) -> u8 {
        if state.inptr < state.insize {
            let byte = state.inbuf[state.inptr];  // Get the byte at the current pointer
            state.inptr += 1;                // Increment the pointer
            byte
        } else {
            self.fill_inbuf(0)  // Call `fill_inbuf(0)` if we're out of buffer
        }
    }

    // `try_byte()` function
    pub fn try_byte(&mut self, state: &mut GzipState) -> u8 {
        if state.inptr < state.insize {
            let byte = state.inbuf[state.inptr];  // Get the byte at the current pointer
            state.inptr += 1;                // Increment the pointer
            byte
        } else {
            self.fill_inbuf(1)  // Call `fill_inbuf(1)` if we're out of buffer
        }
    }

    // Function to get a byte (equivalent to GETBYTE macro)
    pub fn Get_Byte(&mut self, state: &mut GzipState, w: usize) -> u8 {
        if *state.inptr < state.insize {
            let byte = state.inbuf[*state.inptr];
            *state.inptr += 1;
            byte
        } else {
            self.wp = w; // This part needs clarification based on your code
            self.fill_inbuf(0);
            0 // Placeholder, adjust logic as per the context
        }
    }

    // Function to get the next byte
    pub fn next_byte(&mut self, state: &mut GzipState, w: usize) -> u8 {
        self.Get_Byte(state, w)
    }

    // Equivalent to the NEEDBITS macro (requiring more information to be fully accurate)
    pub fn need_bits(&mut self, state: &mut GzipState,k: &mut u32, b: &mut u64, n: u32, w: usize)-> (u32, u64)  {
        while k < n {
                b |= (self.next_byte(state, w) as u64) << k;
                k += 8;
            }
            (k, b)
    }

    // Equivalent to DUMPBITS macro
    pub fn dump_bits(&mut self, k: &mut u32, b: &mut u64, n: u32)-> (u32, u64)  {
        let updated_b = b >> n;
        let updated_k = k - n;
        (updated_k, updated_b)
    }

    // Function to build the Huffman tree
    pub fn huft_build(
        &mut self,
        b: &mut [u32],           // code lengths in bits (all assumed <= BMAX)
        n: u32,                  // number of codes (assumed <= N_MAX)
        s: u32,                  // number of simple-valued codes (0..s-1)
        d: &[u16],           // list of base values for non-simple codes
        e: &[u16],           // list of extra bits for non-simple codes
        t: &mut Option<Box<Huft>>,  // result: starting table
        m: &mut i32,             // maximum lookup bits, returns actual
    ) -> i32 {
        let mut a: u32;          // counter for codes of length k
        let mut c: [u32; BMAX as usize + 1] = [0; BMAX as usize + 1]; // bit length count table
        let mut f: u32;          // i repeats in table every f entries
        let mut g: i32;          // maximum code length
        let mut h: i32;          // table level
        let mut i: u32;          // counter, current code
        let mut j: u32;          // counter
        let mut k: i32;          // number of bits in current code
        let mut l: i32;          // bits per table (returned in m)
        let mut p: usize;        // pointer into c[], b[], or v[]
        let mut q: Option<Box<Huft>>; // points to current table
        let mut r: Huft;         // table entry for structure assignment
        let mut u: [Option<Box<Huft>>; BMAX as usize] = [None; BMAX as usize]; // table stack
        let mut v: [u32; N_MAX as usize] = [0; N_MAX as usize]; // values in order of bit length
        let mut w: i32;          // bits before this table == (l * h)
        let mut x: [u32; BMAX as usize + 1] = [0; BMAX as usize + 1]; // bit offsets, then code stack
        let mut xp: usize;       // pointer into x
        let mut y: u32;          // number of dummy codes added
        let mut z: u32;          // number of entries in current table

        // Generate counts for each bit length
        c.iter_mut().for_each(|el| *el = 0);
        p = 0;
        i = n;
        while i > 0 {
            // Trace output logic here
            c[b[p] as usize] += 1;  // assume all entries <= BMAX
            p += 1;
            i -= 1;
        }
        if c[0] == n {  // null input--all zero length codes
            let q = Box::new(Huft { v: Box::new(None), e: 99, b: 1 });
            self.hufts += 3;
            *t = Some(q);
            *m = 1;
            return 0;
        }

        // Find minimum and maximum length, bound *m by those
        l = *m;
        for j in 1..=BMAX {
            if c[j as usize] > 0 {
                break;
            }
        }
        k = j;
        if l < j {
            l = j;
        }
        for i in (1..=BMAX).rev() {
            if c[i as usize] > 0 {
                break;
            }
        }
        g = i;
        if l > i {
            l = i;
        }
        *m = l;

        // Adjust last length count to fill out codes, if needed
        let mut y = 1 << j;
        for j in k..g {
            if (y -= c[j as usize]) < 0 {
                return 2; // bad input: more codes than bits
            }
        }
        if (y -= c[g as usize]) < 0 {
            return 2;
        }
        c[g as usize] += y;

        // Generate starting offsets into the value table for each length
        x[1] = j;
        p = c.as_mut_ptr();
        xp = &mut x[2];
        while i > 0 {
            *xp += *p;
            p += 1;
        }

        // Generate the Huffman codes and for each, make the table entries
        x[0] = i;
        p = v;
        h = -1;
        w = -l;
        u[0] = None;
        q = None;
        z = 0;

        for k in k..=g {
            a = c[k as usize];
            while a > 0 {
                // Logic to generate the Huffman code
                // fill code-like entries with r, increment the code, etc.
            }
        }

        // Return true (1) if we were given an incomplete table
        return y != 0 && g != 1;
    }



    // Function to inflate coded data
    pub fn inflate_codes(
        &mut self,
        tl: &mut Option<Box<Huft>>, // literal/length code table
        td: &mut Option<Box<Huft>>, // distance code table
        bl: i32,                    // lookup bits for tl
        bd: i32                     // lookup bits for td
    ) -> i32 {
        let mut e: u32;   // table entry flag/number of extra bits
        let mut n: u32;   // length for copy
        let mut d: u32;   // index for copy
        let mut w: usize; // current window position
        let mut t: Option<Box<Huft>>;  // pointer to table entry
        let mut ml: u32;  // masks for bl
        let mut md: u32;  // masks for bd
        let mut b: u64;   // bit buffer
        let mut k: u32;   // number of bits in bit buffer

        // Make local copies of globals
        b = unsafe { self.bb }; // initialize bit buffer
        k = unsafe { self.bk };
        w = unsafe { self.wp }; // initialize window position

        // Inflate the coded data
        ml = mask_bits[bl as usize]; // precompute masks for speed
        md = mask_bits[bd as usize];

        loop {
            let (k, b) = self.need_bits(state, &mut k, &mut b, bl as u32, w);
            e = (t = tl + ((b & ml) as usize)).e;
            if e > 16 {
                loop {
                    if e == 99 {
                        return 1;
                    }
                    let (k, b) = self.dump_bits(&mut k, &mut b, t.b);
                    e -= 16;
                    let (k, b) = self.need_bits(state, &mut k, &mut b, e, w);
                }
            }
            let (k, b) = self.dump_bits(&mut k, &mut b, t.b);

            if e == 16 {  // then it's a literal
                self.slide[w] = t.v.n;
                eprintln!("%c", self.slide[w - 1]);
                if w == WSIZE {
                    self.flush_output(state, w);
                    w = 0;
                }
            } else {  // it's an EOB or a length
                if e == 15 {
                    break;  // exit if end of block
                }

                // Get the length of block to copy
                let (k, b) = self.need_bits(state, &mut k, &mut b, e, w);
                n = t.v.n + (b & mask_bits[e]);
                let (k, b) = self.dump_bits(&mut k, &mut b, e);

                // Decode distance of block to copy
                let (k, b) = self.need_bits(state, &mut k, &mut b, bd as u32, w);
                if (e = (t = td + (b & md)).e) > 16 {
                    loop {
                        if e == 99 {
                            return 1;
                        }
                        let (k, b) = self.dump_bits(&mut k, &mut b, t.b);
                        e -= 16;
                        let (k, b) = self.need_bits(state, &mut k, &mut b, e, w);
                    }
                }
                let (k, b) = self.dump_bits(&mut k, &mut b, t.b);
                let (k, b) = self.need_bits(state, &mut k, &mut b, e, w);
                d = w - t.v.n - (b & mask_bits[e]);
                let (k, b) = self.dump_bits(&mut k, &mut b, e);
                eprintln!("\\[%d,%d]", w - d, n);

                // Do the copy
                loop {
                    n -= e;
                    if e <= (d < w ? w - d : d - w) {
                        self.slide.copy_from_slice(self.slide + w, self.slide + d, e);
                        w += e;
                        d += e;
                    } else {
                        loop {
                            self.slide[w] = self.slide[d];
                            eprintln!("%c", self.slide[w - 1]);
                            d += 1;
                        }
                    }
                }
                if w == WSIZE {
                    self.flush_output(state, w);
                    w = 0;
                }
            }
        }

        // Restore the globals from the locals
        self.wp = w;
        self.bb = b;
        self.bk = k;

        // Done
        return 0;
    }

    // Function to decompress an inflated type 0 (stored) block.
    pub fn inflate_stored(&mut self) -> i32 {
        let mut n: u32;           // number of bytes in block
        let mut w: usize;         // current window position
        let mut b: u64;           // bit buffer
        let mut k: u32;           // number of bits in bit buffer

        // Make local copies of globals
        b = unsafe { self.bb };
        k = unsafe { self.bk };
        w = unsafe { self.wp };

        // Go to byte boundary
        n = k & 7;
        let (k, b) = self.dump_bits(&mut k, &mut b, n);

        // Get the length and its complement
        let (k, b) = self.need_bits(state, &mut k, &mut b, 16, w);
        n = (b & 0xffff);
        let (k, b) = self.dump_bits(&mut k, &mut b, 16);
        let (k, b) = self.need_bits(state, &mut k, &mut b, 16, w);
        if n != !(b & 0xffff) {
            return 1;  // error in compressed data
        }
        let (k, b) = self.dump_bits(&mut k, &mut b, 16);

        // Read and output the compressed data
        while n > 0 {
            let (k, b) = self.need_bits(state, &mut k, &mut b, 8, w);
            self.slide[w] = b;
            if w == WSIZE {
                self.flush_output(state, w);
                w = 0;
            }
            let (k, b) = self.dump_bits(&mut k, &mut b, 8);
        }

        // Restore the globals from the locals
        self.wp = w;
        self.bb = b;
        self.bk = k;
        return 0;
    }

    // Decompress an inflated type 1 (fixed Huffman codes) block
    pub fn inflate_fixed(&mut self) -> i32 {
        let mut i: i32;  // temporary variable
        let mut tl: Option<Box<Huft>>;  // literal/length code table
        let mut td: Option<Box<Huft>>;  // distance code table
        let mut bl: i32;  // lookup bits for tl
        let mut bd: i32;  // lookup bits for td
        let mut l: [u32; 288]; // length list for huft_build

        // Set up literal table
        for i in 0..144 {
            l[i] = 8;
        }
        for i in 144..256 {
            l[i] = 9;
        }
        for i in 256..280 {
            l[i] = 7;
        }
        for i in 280..288 {
            l[i] = 8;  // make a complete, but wrong code set
        }
        bl = 7;
        if self.huft_build(&mut l, 288, 257, &cplens, &cplext, &mut tl, &bl) != 0 {
            return 1;
        }

        // Set up distance table
        for i in 0..30 {
            l[i] = 5;  // make an incomplete code set
        }
        bd = 5;
        if self.huft_build(&mut l, 30, 0, &cpdist, &cpdext, &mut td, &bd) > 1 {
            huft_free(tl);
            return 1;
        }

        // Decompress until an end-of-block code
        if self.inflate_codes(tl, td, bl, bd) != 0 {
            return 1;
        }

        // Free the decoding tables and return
        huft_free(tl);
        huft_free(td);
        return 0;
    }

    // Decompress an inflated type 2 (dynamic Huffman codes) block
    // Decompress an inflated type 2 (dynamic Huffman codes) block.
    pub fn inflate_dynamic(&mut self) -> i32 {
        let mut i: u32;  // temporary variables
        let mut j: u32;
        let mut l: u32;  // last length
        let mut m: u32;  // mask for bit lengths table
        let mut n: u32;  // number of lengths to get
        let mut w: usize; // current window position
        let mut tl: Option<Box<Huft>>;  // literal/length code table
        let mut td: Option<Box<Huft>>;  // distance code table
        let mut bl: i32;  // lookup bits for tl
        let mut bd: i32;  // lookup bits for td
        let mut nb: u32;  // number of bit length codes
        let mut nl: u32;  // number of literal/length codes
        let mut nd: u32;  // number of distance codes
        let mut ll: [u32; 286 + 30];  // literal/length and distance code lengths
        let mut b: u32;  // bit buffer
        let mut k: u32;  // number of bits in bit buffer

        // Make local bit buffer
        b = unsafe { self.bb };
        k = unsafe { self.bk };
        w = unsafe { self.wp };

        // Read in table lengths
        let (k, b) = self.need_bits(state, &mut k, &mut b, 5, w);
        nl = 257 + (b & 0x1f); // number of literal/length codes
        let (k, b) = self.dump_bits(&mut k, &mut b, 5);
        let (k, b) = self.need_bits(state, &mut k, &mut b, 5, w);
        nd = 1 + (b & 0x1f); // number of distance codes
        let (k, b) = self.dump_bits(&mut k, &mut b, 5);
        let (k, b) = self.need_bits(state, &mut k, &mut b, 4, w);
        nb = 4 + (b & 0xf); // number of bit length codes
        let (k, b) = self.dump_bits(&mut k, &mut b, 4);
        if nl > 286 || nd > 30 {
            return 1;  // bad lengths
        }

        // Read in bit-length-code lengths
        for j in 0..nb {
            let (k, b) = self.need_bits(state, &mut k, &mut b, 4, w);
            ll[border[j as usize]] = b & 7;
            let (k, b) = self.dump_bits(&mut k, &mut b, 3);
        }
        for j in nb..19 {
            ll[border[j as usize]] = 0;
        }

        // Build decoding table for trees - single level, 7 bit lookup
        bl = 7;
        if (i = self.huft_build(&mut ll, 19, 19, None, None, &mut tl, &mut bl)) != 0 {
            if i == 1 {
                huft_free(tl);
            }
            return i;  // incomplete code set
        }

        if tl == None {
            return 2;
        }

        // Read in literal and distance code lengths
        n = nl + nd;
        m = mask_bits[bl as usize];
        i = 0;
        while i < n as u32 {
            let (k, b) = self.need_bits(state, &mut k, &mut b, bl as u32, w);
            j = (td = tl + (b & m)).b;
            let (k, b) = self.dump_bits(&mut k, &mut b, j);
            if td.unwrap().e == 99 {
                // Invalid code.
                huft_free(tl);
                return 2;
            }
            j = td.unwrap().v.n;
            if j < 16 { // length of code in bits (0..15)
                l = j;                    // Save the last length in `l`
                ll[i as usize] = l;       // Assign `l` to `ll[i]`

            } else if j == 16 {  // repeat last length 3 to 6 times
                let (k, b) = self.need_bits(state, &mut k, &mut b, 2, w);
                j = 3 + ((b & 3)as u32);
                let (k, b) = self.dump_bits(&mut k, &mut b, 2);
                if i + j > n {
                    return 1;
                }
                while j > 0 {
                    ll[i as usize] = l;
                    i += 1;
                    j -= 1;
                }
            } else if j == 17 {  // 3 to 10 zero length codes
                let (k, b) = self.need_bits(state, &mut k, &mut b, 3, w);
                j = 3 + ((b & 7)as u32);
                let (k, b) = self.dump_bits(&mut k, &mut b, 3);
                if i + j > n {
                    return 1;
                }
                while j > 0 {
                    ll[i as usize] = 0;
                    i += 1;
                    j -= 1;
                }
                l = 0;
            } else { // j == 18: 11 to 138 zero length codes
                let (k, b) = self.need_bits(state, &mut k, &mut b, 7, w);
                j = 11 + ((b & 0x7f)as u32);
                let (k, b) = self.dump_bits(&mut k, &mut b, 7);
                if i + j > n {
                    return 1;
                }
                while j > 0 {
                    ll[i as usize] = 0;
                    i += 1;
                    j -= 1;
                }
                l = 0;
            }
        }

        // Free decoding table for trees
        huft_free(tl);

        // Restore the global bit buffer
        self.bb = b;
        self.bk = k;

        // Build the decoding tables for literal/length and distance codes
        bl = self.lbits;
        let i = self.huft_build(&mut ll, nl, 257, &cpdist, &cpdext, &mut tl, &mut bl);
        if i != 0 {
            if i == 1 {
                eprintln!("incomplete literal tree\n");
                huft_free(tl);
            }
            return i;  // incomplete code set
        }
        bd = self.dbits;
        let i = self.huft_build(&mut ll, nd, 0, &cpdist, &cpdext, &mut td, &mut bd);
        if i != 0 {
            if i == 1 {
                eprintln!("incomplete distance tree\n");
                huft_free(td);
            }
            huft_free(tl);
            return i;  // incomplete code set
        }

        // Decompress until an end-of-block code
        let err = self.inflate_codes(&mut tl, &mut td, bl, bd);
        // Free the decoding tables
        huft_free(tl);
        huft_free(td);

        return err;
    }


    // Decompress an inflated block
    // Decompress an inflated block
    // E is the last block flag
    pub fn inflate_block(&mut self, e: &mut i32) -> i32 {
        let mut t: u32;  // block type
        let mut w: usize;  // current window position
        let mut b: u64;  // bit buffer
        let mut k: u32;  // number of bits in bit buffer

        // Make local bit buffer
        b = unsafe { self.bb };  // initialize bit buffer
        k = unsafe { self.bk };
        w = unsafe { self.wp };  // initialize window position

        // Read in last block bit
        let (k, b) = self.need_bits(state, &mut k, &mut b, 1, w);
        *e = (b & 1) as i32;
        let (k, b) = self.dump_bits(&mut k, &mut b, 1);

        // Read in block type
        let (k, b) = self.need_bits(state, &mut k, &mut b, 2, w);
        t = (b & 3) as u32;
        let (k, b) = self.dump_bits(&mut k, &mut b, 2);

        // Restore the global bit buffer
        unsafe {
            self.bb = b;
            self.bk = k;
        }

        // Inflate that block type
        if t == 2 {
            return self.inflate_dynamic();
        }
        if t == 0 {
            return self.inflate_stored();
        }
        if t == 1 {
            return self.inflate_fixed();
        }

        // Bad block type
        return 2;
    }


    // Decompress an inflated entry
    pub fn inflate(&mut self, state: &mut GzipState) -> i32 {
        let mut e: i32;  // last block flag
        let mut r: i32;  // result code
        let mut h: u32;  // maximum struct huft's malloc'ed

        // Initialize window, bit buffer
        self.wp = 0;
        self.bk = 0;
        self.bb = 0;

        // Decompress until the last block
        h = 0;
        loop {
            self.hufts = 0;
            r = self.inflate_block(&mut e);
            if r != 0 {
                return r;
            }
            if self.hufts > h {
                h = self.hufts;
            }
            if e != 0 {
                break;  // Exit when the last block is reached
            }
        }

        // Undo too much lookahead. The next read will be byte aligned so we
        // can discard unused bits in the last meaningful byte.
        while self.bk >= 8 {
            self.bk -= 8;
            state.inptr -= 1;
        }

        // Flush out slide
        self.flush_output(state, self.wp);

        // Return success
        eprintln!(&format!("<{}> ", h));
        return 0;
    }
}













