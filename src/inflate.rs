use std::io;
use std::ptr::null_mut;
use crate::GzipState;
use crate::trees::Trees;
use crate::{OK, ERROR, STORED, WSIZE, INBUFSIZ};
use std::io::{stdout, Read, Write, Cursor};

#[derive(Debug)]
#[derive(Clone)]
struct Huft {
    v: HuftValue, // Pointer to next level of table or value
    e: u8, // Extra bits for the current table
    b: u8, // Number of bits for this code or subcode
}

#[derive(Debug)]
#[derive(Clone)]
enum HuftValue {
    N(u16),          // Literal, length base, or distance base
    T(Box<[Huft]>),  // Pointer to a fixed-size array (dynamic allocation)
}

impl Default for HuftValue {
    fn default() -> Self {
        HuftValue::N(0)
    }
}

impl Default for Huft {
    fn default() -> Self {
        Huft {
            v: HuftValue::default(),
            e: 0,
            b: 0,
        }
    }
}

fn huft_free(t: Option<&Huft>) -> usize {
    if let Some(huft) = t {
        match &huft.v {
            HuftValue::T(sub_table) => {
                // 遍历子表并递归计算释放的节点数量
                sub_table.iter().map(|sub_huft| huft_free(Some(sub_huft))).sum::<usize>() + 1
            }
            HuftValue::N(_) => 1, // 叶子节点直接返回 1
        }
    } else {
        0 // None 时返回 0
    }
}


fn find_huft_entry(current: &Huft, index: usize) -> Option<&Huft> {
    match &current.v {
        HuftValue::T(sub_table) => sub_table.get(index),
        HuftValue::N(_) => None,
    }
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
static mask_bits: [u32; 17] = [
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
    bb: u32,
    bk: u32,
    wp: usize,
    lbits: i32,
    dbits: i32,
    hufts: u32,
    slide: [u8; 2 * WSIZE],
}

impl Inflate {
    pub fn new() -> Self {
        Self {
            bb: 0,
            bk: 0,
            wp: 0,
            lbits: 9,
            dbits: 6,
            hufts: 0,
            slide: [0; 2 * WSIZE],
        }
    }

    pub fn fill_inbuf<R: Read>(&mut self, input: &mut R, eof_ok: bool, state: &mut GzipState) -> io::Result<u8> {
        state.insize = 0;
        loop {
            let len = self.read_buffer(input, state)?;
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
                return Ok(0xFF);
            }
            self.flush_window(state)?;
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "Unexpected EOF"));
        }
        state.bytes_in += state.insize as i64;
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

    pub fn flush_window(&mut self, state: &mut GzipState) -> std::io::Result<()> {
        if state.outcnt == 0 {
            return Ok(());
        }

        state.updcrc(Some(&state.window.clone()), state.outcnt);

        if !state.test {
            state.ofd.as_mut().expect("REASON").write_all(&state.window[0..state.outcnt])?;
//             state.write_buf(&mut state.ofd, &state.window[0..state.outcnt], state.outcnt);
        }

        state.bytes_out += state.outcnt as i64;
        state.outcnt = 0;
        Ok(())
    }

    // Function to flush output (equivalent to macro flush_output in C)
    pub fn flush_output(&mut self, state: &mut GzipState, w: usize) {
        unsafe {
            self.wp = w;
        }
        self.flush_window(state);
    }

    pub fn get_byte(&mut self, state: &mut GzipState) -> io::Result<u8> {
        if state.inptr < state.insize {
            let byte = state.inbuf[state.inptr];  // Get the byte at the current pointer
            state.inptr += 1;                // Increment the pointer
            Ok(byte)
        } else {
            let mut input = Cursor::new(vec![0; 1]);
            Ok(self.fill_inbuf(&mut input, true, state)?)
        }
    }

    // `try_byte()` function
    pub fn try_byte(&mut self, state: &mut GzipState) -> io::Result<u8> {
        if state.inptr < state.insize {
            let byte = state.inbuf[state.inptr];  // Get the byte at the current pointer
            state.inptr += 1;                // Increment the pointer
            Ok(byte)
        } else {
            let mut input = Cursor::new(vec![1; 1]);
            Ok(self.fill_inbuf(&mut input, true, state)?)
        }
    }

    // Function to get a byte (equivalent to GETBYTE macro)
    pub fn Get_Byte(&mut self, state: &mut GzipState, w: usize) -> io::Result<u8> {
        if state.inptr < state.insize {
            let byte = state.inbuf[state.inptr];
            state.inptr += 1;
            Ok(byte)
        } else {
            self.wp = w; // This part needs clarification based on your code
            let mut input = Cursor::new(vec![0; 1]);
            self.fill_inbuf(&mut input, true, state)?;
            Ok(0) // Placeholder, adjust logic as per the context
        }
    }

    // Function to get the next byte
    pub fn next_byte(&mut self, state: &mut GzipState, w: usize) -> io::Result<u8> {
        self.Get_Byte(state, w)
    }

    // Equivalent to the NEEDBITS macro (requiring more information to be fully accurate)
    pub fn need_bits(&mut self, state: &mut GzipState,k: &mut u32, b: &mut u32, n: u32, w: usize)  {
        while *k < n {
                let byte = match self.next_byte(state, w) {
                    Ok(value) => value, Err(_) => todo!(),
                };
                *b |= (u32::from(byte)) << *k;

                *k += 8;
            }
    }

    // Equivalent to DUMPBITS macro
    pub fn dump_bits(&mut self, k: &mut u32, b: &mut u32, n: u32)  {
        *b = *b >> n;
        *k = *k - n;
    }

    // Function to build the Huffman tree
    pub fn huft_build(
        &mut self,
        b: &[u32],   // Code lengths in bits
        n: usize,    // Number of codes
        s: usize,    // Number of simple-valued codes (0..s-1)
        d: &[u16],   // List of base values for non-simple codes
        e: &[u16],   // List of extra bits for non-simple codes
        t: &mut Option<Box<Huft>>, // Result: starting table
        m: &mut i32, // Maximum lookup bits, returns actual
    ) -> u32 {
        let mut c = [0u32; BMAX as usize + 1];
        let mut x = [0u32; BMAX as usize + 1];
        let mut v = [0u32; N_MAX as usize];
        let mut u: Vec<Box<[Huft]>> = Vec::new(); // Replace linked list with dynamic array

        let mut a;
        let mut f: u32;
        let mut g: u32;
        let mut h: i32 = -1;
        let mut l = *m;
        let mut w = 0;

        // Generate counts for each bit length
        for &bit in b.iter().take(n) {
            c[bit as usize] += 1;
        }

        if c[0] == n as u32 {
            *t = Some(Box::new(Huft {
                v: HuftValue::T(Box::new([])),
                e: 99,
                b: 1,
            }));
            *m = 1;
            return 0;
        }

        // Find minimum and maximum length
        let mut k = c.iter().position(|&x| x != 0).unwrap_or(0) as i32;
        g = (1..=BMAX).rev().find(|&x| c[x as usize] != 0).unwrap_or(0) as u32 ;
        l = l.clamp(k, g as i32);
        *m = l;

        // Adjust last length count
        let mut y = 1 << k;
        for j in k..g as i32 {
            y -= c[j as usize];
            if y < 0 {
                return 2;
            }
            y <<= 1;
        }

        y -= c[g as usize];
        if y < 0 {
            return 2;
        }
        c[g as usize] += y;

        // Generate starting offsets
        let mut j = 0;
        for i in 1..=BMAX {
            x[i as usize] = j;
            j += c[i as usize] as u32;
        }

        // Populate values array
        for (i, &bit) in b.iter().enumerate().take(n) {
            if bit != 0 {
                v[x[bit as usize] as usize] = i as u32;
                x[bit as usize] += 1;
            }
        }

        let n = x[g as usize] as usize;

        // Generate Huffman codes and build tables
        x[0] = 0;
        let mut p = v.iter();
        let mut z = 0;

        for k in k..=g as i32 {
            a = c[k as usize];
            while a > 0 {
                while k > w + l {
                    h += 1;
                    w += l;

                    let z = (g - w as u32).clamp(1, l as u32) as usize;
                    let f = 1 << z;

                    // Allocate subtable as an array
                    let subtable = vec![Huft::default(); f].into_boxed_slice();
                    u.push(subtable);
                }

                let mut r = Huft::default();
                r.b = (k - w) as u8;

                if let Some(&val) = p.next() {
                    if val < s as u32 {
                        r.e = if val < 256 { 16 } else { 15 };
                        r.v = HuftValue::N(val as u16);
                    } else {
                        r.e = e[val as usize - s] as u8;
                        r.v = HuftValue::N(d[val as usize - s]);
                    }
                } else {
                    r.e = 99;
                }

                let mut f = 1 << (k - w);
                let base_index = x[(k - w) as usize] as usize;

                if let Some(subtable) = u.last_mut() {
                    for i in 0..f {
                        subtable[base_index + i] = r.clone();
                    }
                }

                a -= 1;
            }
        }

        // Set result table
        if let Some(subtable) = u.pop() {
            *t = Some(Box::new(Huft {
                v: HuftValue::T(subtable),
                e: 0,
                b: 0,
            }));
        }

        (y != 0 && g != 1) as u32
    }

    // Function to inflate coded data
    pub fn inflate_codes(
        &mut self,
        state: &mut GzipState,
        tl: &Option<Box<Huft>>, // Literal/length table
        td: &Option<Box<Huft>>, // Distance table
        bl: i32,                // Number of bits for literal/length table
        bd: i32,                // Number of bits for distance table
    ) -> i32 {
        let mut b = self.bb; // Bit buffer
        let mut k = self.bk; // Number of bits in bit buffer
        let mut w = self.wp; // Current window position

        let ml = mask_bits[bl as usize]; // Mask for `bl` bits
        let md = mask_bits[bd as usize]; // Mask for `bd` bits

        loop {
            // Get a literal/length code
            self.need_bits(state, &mut k, &mut b, bl as u32, w);
            let index = (b & ml) as usize;

            // Traverse the literal/length table
            let mut t = match tl {
                Some(t) => &**t,
                None => return 2,
            };
            while let HuftValue::T(ref table) = t.v {
                t = &table[index];
            }

            let mut e = t.e;
            while e > 16 {
                if e == 99 {
                    return 1; // Invalid code
                }
                self.dump_bits(&mut k, &mut b, t.b as u32);
                e -= 16;

                self.need_bits(state, &mut k, &mut b, e as u32, w);
                let index = (b & mask_bits[e as usize]) as usize;

                if let HuftValue::T(ref table) = t.v {
                    t = &table[index];
                } else {
                    return 2; // Invalid structure
                }
                e = t.e;
            }

            self.dump_bits(&mut k, &mut b, t.b as u32);

            if e == 16 {
                // Literal
                let n = match t.v {
                    HuftValue::N(n) => n as usize,
                    _ => panic!("Expected HuftValue::N, but found HuftValue::T"),
                };
                self.slide[w] = n as u8;
                w += 1;
                if w == WSIZE {
                    self.flush_output(state, w);
                    w = 0;
                }
            } else {
                // End of block or length
                if e == 15 {
                    break; // End of block
                }

                // Get length of block to copy
                self.need_bits(state, &mut k, &mut b, e as u32, w);
                let mut n = match t.v {
                    HuftValue::N(n) => n as usize + (b & mask_bits[e as usize]) as usize,
                    _ => panic!("Expected HuftValue::N, but found HuftValue::T"),
                };
                self.dump_bits(&mut k, &mut b, e as u32);

                // Get distance of block to copy
                self.need_bits(state, &mut k, &mut b, bd as u32, w);
                let index = (b & md) as usize;

                // Traverse the distance table
                let mut t = match td {
                    Some(t) => &**t,
                    None => return 2,
                };
                while let HuftValue::T(ref table) = t.v {
                    t = &table[index];
                }

                let mut e = t.e;
                while e > 16 {
                    if e == 99 {
                        return 1; // Invalid code
                    }
                    self.dump_bits(&mut k, &mut b, t.b as u32);
                    e -= 16;

                    self.need_bits(state, &mut k, &mut b, e as u32, w);
                    let index = (b & mask_bits[e as usize]) as usize;

                    if let HuftValue::T(ref table) = t.v {
                        t = &table[index];
                    } else {
                        return 2; // Invalid structure
                    }
                    e = t.e;
                }

                self.dump_bits(&mut k, &mut b, t.b as u32);

                self.need_bits(state, &mut k, &mut b, e as u32, w);
                let mut d = match t.v {
                    HuftValue::N(n) => w as isize - n as isize - (b & mask_bits[e as usize]) as isize,
                    _ => panic!("Expected HuftValue::N, but found HuftValue::T"),
                };
                self.dump_bits(&mut k, &mut b, e as u32);

                // Copy block
                while n > 0 {
                    let e = (if d >= 0 {
                        WSIZE - d as usize
                    } else {
                        w - d as usize
                    })
                    .min(n);

                    if d >= 0 && d + e as isize <= w as isize {
                        self.slide.copy_within(d as usize..d as usize + e, w);
                        w += e;
                        d += e as isize;
                    } else {
                        for _ in 0..e {
                            self.slide[w] = self.slide[d as usize];
                            w += 1;
                            d += 1;
                        }
                    }
                    n -= e;

                    if w == WSIZE {
                        self.flush_output(state, w);
                        w = 0;
                    }
                }
            }
        }

        // Restore globals
        self.wp = w;
        self.bb = b;
        self.bk = k;

        0 // Success
    }

    // Function to decompress an inflated type 0 (stored) block.
    pub fn inflate_stored(&mut self, state: &mut GzipState) -> i32 {
        let mut n: u32;          // number of bytes in block
        let mut w: usize;        // current window position
        let mut b: u32;          // bit buffer
        let mut k: u32;          // number of bits in bit buffer

        // make local copies of globals
        b = self.bb;  // initialize bit buffer
        k = self.bk;  // number of bits in bit buffer
        w = self.wp;  // initialize window position

        // go to byte boundary
        n = k & 7;
        self.dump_bits(&mut k, &mut b, n);

        // get the length and its complement
        self.need_bits(state, &mut k, &mut b, 16, w);
        n = (b & 0xffff) as u32;
        self.dump_bits(&mut k, &mut b, 16);
        self.need_bits(state, &mut k, &mut b, 16,w);

        if n != (!b & 0xffff) as u32 {
            return 1;  // error in compressed data
        }
        self.dump_bits(&mut k, &mut b, 16);

        // read and output the compressed data
        while n > 0 {
            self.need_bits(state, &mut k, &mut b, 8, w);
            self.slide[w] = (b & 0xff) as u8;  // assuming slide is an array
            w += 1;

            if w == WSIZE {
                self.flush_output(state, w);
                w = 0;
            }
            self.dump_bits(&mut k, &mut b, 8);
            n -= 1;
        }

        // restore the globals from the locals
        self.wp = w;  // restore global window pointer
        self.bb = b;  // restore global bit buffer
        self.bk = k;

        return 0;
    }

    // Decompress an inflated type 1 (fixed Huffman codes) block
    pub fn inflate_fixed(&mut self, state: &mut GzipState) -> i32 {
        let mut tl: Option<Box<Huft>> = None; // Literal/length table
        let mut td: Option<Box<Huft>> = None; // Distance table
        let mut bl: i32 = 7;                 // Lookup bits for `tl`
        let mut bd: i32 = 5;                 // Lookup bits for `td`
        let mut l = [0u32; 288];             // Length list for `huft_build`

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
            l[i] = 8;
        }

        // Call huft_build for literal/length table
        let result = self.huft_build(&l, 288, 257, &cplens, &cplext, &mut tl, &mut bl);
        if result != 0 {
            return result as i32;
        }

        // Set up distance table
        let mut l = [0u32; 30]; // Length list for distance table
        for i in 0..30 {
            l[i] = 5;
        }

        // Call huft_build for distance table
        let result = self.huft_build(&l, 30, 0, &cpdist, &cpdext, &mut td, &mut bd);
        println!("fixed!");
        if result != 0 {
            if let Some(ref tl) = tl {
                huft_free(Some(tl));
            }
            return result as i32;
        }

        // Decompress until an end-of-block code
        if self.inflate_codes(state, &mut tl, &mut td, bl, bd) != 0 {
            return 1;
        }

        // Free the decoding tables
        if let Some(ref tl) = tl {
            huft_free(Some(tl));
        }
        if let Some(ref td) = td {
            huft_free(Some(td));
        }

        0
    }



    // Decompress an inflated type 2 (dynamic Huffman codes) block
    pub fn inflate_dynamic(&mut self, state: &mut GzipState) -> i32 {
        let mut tl: Option<Box<Huft>> = None; // Literal/length table
        let mut td: Option<Box<Huft>> = None; // Distance table
        let mut bl: i32 = 7;                 // Lookup bits for `tl`
        let mut bd: i32 = 5;                 // Lookup bits for `td`
        let mut b = self.bb;                 // Bit buffer
        let mut k = self.bk;                 // Number of bits in the bit buffer
        let mut w = self.wp as u32;          // Current window position

        // Read table lengths
        self.need_bits(state, &mut k, &mut b, 5, w as usize);
        let nl = 257 + (b & 0x1f); // Number of literal/length codes
        self.dump_bits(&mut k, &mut b, 5);
        self.need_bits(state, &mut k, &mut b, 5, w as usize);
        let nd = 1 + (b & 0x1f);   // Number of distance codes
        self.dump_bits(&mut k, &mut b, 5);
        self.need_bits(state, &mut k, &mut b, 4, w as usize);
        let nb = 4 + (b & 0xf);    // Number of bit length codes
        self.dump_bits(&mut k, &mut b, 4);

        if nl > 286 || nd > 30 {
            return 1; // Invalid code lengths
        }

        // Build bit-length table
        let mut bit_lengths = vec![0u32; 19];
        for j in 0..nb {
            self.need_bits(state, &mut k, &mut b, 3, w as usize);
            bit_lengths[border[j as usize] as usize] = b & 7;
            self.dump_bits(&mut k, &mut b, 3);
        }

        // Set remaining lengths to zero
        for j in nb..19 {
            bit_lengths[border[j as usize] as usize] = 0;
        }

        // Build the Huffman table for bit-length codes
        let mut result = self.huft_build(&bit_lengths, 19, 19, &[], &[], &mut tl, &mut bl);
        if result != 0 {
            if result == 1 {
                if let Some(ref tl) = tl {
                    huft_free(Some(tl));
                }
            }
            return result as i32;
        }

        if tl.is_none() {
            return 2; // Error in tree decoding
        }

        // Decode literal/length and distance code lengths
        let n = nl + nd;
        let mut literal_lengths = vec![0u32; n as usize];
        let mut i = 0;
        let mut l = 0;
        let mask = mask_bits[bl as usize];

        while i < n {
            self.need_bits(state, &mut k, &mut b, bl as u32, w as usize);
            let index = (b & mask) as usize;

            let entry = match tl.as_ref() {
                Some(table) => {
                    let mut t = &**table;
                    while let HuftValue::T(ref subtable) = t.v {
                        t = &subtable[index];
                    }
                    t
                }
                None => return 2,
            };

            self.dump_bits(&mut k, &mut b, entry.b as u32);

            if entry.e == 99 {
                if let Some(ref tl) = tl {
                    huft_free(Some(tl));
                }
                return 2; // Invalid code
            }

            let j = match entry.v {
                HuftValue::N(value) => value as u32,
                _ => return 2, // Unexpected value type
            };

            if j < 16 {
                l = j;
                literal_lengths[i as usize] = l;
                i += 1;
            } else if j == 16 {
                self.need_bits(state, &mut k, &mut b, 2, w as usize);
                let repeat = 3 + (b & 3);
                self.dump_bits(&mut k, &mut b, 2);
                if i + repeat > n {
                    return 1; // Invalid repeat
                }
                for _ in 0..repeat {
                    literal_lengths[i as usize] = l;
                    i += 1;
                }
            } else if j == 17 {
                self.need_bits(state, &mut k, &mut b, 3, w as usize);
                let repeat = 3 + (b & 7);
                self.dump_bits(&mut k, &mut b, 3);
                if i + repeat > n {
                    return 1; // Invalid repeat
                }
                for _ in 0..repeat {
                    literal_lengths[i as usize] = 0;
                    i += 1;
                }
                l = 0;
            } else if j == 18 {
                self.need_bits(state, &mut k, &mut b, 7, w as usize);
                let repeat = 11 + (b & 0x7f);
                self.dump_bits(&mut k, &mut b, 7);
                if i + repeat > n {
                    return 1; // Invalid repeat
                }
                for _ in 0..repeat {
                    literal_lengths[i as usize] = 0;
                    i += 1;
                }
                l = 0;
            }
        }

        // Free the bit-length table
        if let Some(ref tl) = tl {
            huft_free(Some(tl));
        }

        // Restore the global bit buffer
        self.bb = b;
        self.bk = k;

        // Build literal/length and distance Huffman tables
        bl = self.lbits;
        result = self.huft_build(&literal_lengths, nl as usize, 257, &cpdist, &cpdext, &mut tl, &mut bl);
        if result != 0 {
            if result == 1 {
                if let Some(ref tl) = tl {
                    huft_free(Some(tl));
                }
            }
            return result as i32;
        }

        bd = self.dbits;
        result = self.huft_build(
            &literal_lengths[nl as usize..],
            nd as usize,
            0,
            &cpdist,
            &cpdext,
            &mut td,
            &mut bd,
        );
        if result != 0 {
            if result == 1 {
                if let Some(ref td) = td {
                    huft_free(Some(td));
                }
            }
            if let Some(ref tl) = tl {
                huft_free(Some(tl));
            }
            return result as i32;
        }

        // Decompress until an end-of-block code
        println!("dynamic!");
        let err = if self.inflate_codes(state, &mut tl, &mut td, bl, bd) > 0 {
            1
        } else {
            0
        };

        // Free decoding tables
        if let Some(ref tl) = tl {
            huft_free(Some(tl));
        }
        if let Some(ref td) = td {
            huft_free(Some(td));
        }
        err
    }




    // Decompress an inflated block
    // E is the last block flag
    pub fn inflate_block(&mut self, e: &mut i32, state: &mut GzipState) -> i32 {
        let mut t: u32;        // Block type
        let mut w: u32;        // Current window position
        let mut b: u32;        // Bit buffer
        let mut k: u32;        // Number of bits in the bit buffer

        // Initialize local variables
        b = self.bb;
        k = self.bk;
        w = self.wp as u32;

        // Read the last block bit
        self.need_bits(state, &mut k, &mut b, 1, w.try_into().unwrap());
        *e = (b & 1) as i32;
        self.dump_bits(&mut k, &mut b, 1);

        // Read the block type
        self.need_bits(state, &mut k, &mut b, 2, w.try_into().unwrap());
        t = (b & 3) as u32;
        self.dump_bits(&mut k, &mut b, 2);

        // Restore the global bit buffer
        self.bb = b;
        self.bk = k;

        // Decompress based on the block type
        match t {
            2 => return self.inflate_dynamic(state),
            0 => return self.inflate_stored(state),
            1 => return self.inflate_fixed(state),
            _ => return 2, // Invalid block type
        }
    }


    // Decompress an inflated entry
    pub fn inflate(&mut self, state: &mut GzipState) -> i32 {
        let mut e: i32 = 42; // Last block flag
        let mut r: i32; // Result code
        let mut h: u32; // Maximum number of `huft` structures allocated

        // Initialize the window and bit buffer
        self.wp = 0; // Current window position
        self.bk = 0; // Number of bits in the bit buffer
        self.bb = 0; // Bit buffer

        // Decompress until the last block
        h = 0;
        loop {
            self.hufts = 0; // Initialize `hufts`

            r = self.inflate_block(&mut e, state);
            if r != 0 {
                return r; // Return the error code
            }

            if self.hufts > h {
                h = self.hufts; // Update the maximum `hufts`
            }

            if e != 0 {
                break; // Exit the loop if this is the last block
            }
        }

        // Undo excess pre-reading. The next read will be byte-aligned,
        // so discard unused bits from the last meaningful byte.

        while self.bk >= 8 {
            self.bk -= 8;
            state.inptr -= 1; // Assume `inptr` is a global variable pointing to the input buffer
        }

        // Flush the output window
        self.flush_output(state, self.wp); // Assume `flush_output` is a function that writes decompressed data to the output

        // Return success status
        println!("{}", format!("<{}> ", h)); // Assume `trace` is a debugging output function
        0
    }
}













