use std::io;
use std::ptr::null_mut;
use crate::GzipState;
use crate::trees::Trees;
use crate::{OK, ERROR, STORED, WSIZE, INBUFSIZ};
use std::io::{stdout, Read, Write, Cursor};

#[derive(Debug)]
#[derive(Clone)] // 自动为 Huft 实现 Clone 特性
struct Huft {
    v: Box<Option<Box<Huft>>>, // Pointer to next level of table or value
    e: u8, // Extra bits for the current table
    b: u8, // Number of bits for this code or subcode
}

enum HuftValue {
    N(u16),         // literal, length base, or distance base
    T(Box<Huft>),   // pointer to next level of table
}

impl Default for Huft {
    fn default() -> Self {
        Huft {
            v: Box::new(None),  // 假设 Value::N 是一个枚举类型，可以用一个默认值
            b: 0,             // u8 的默认值是 0
            e: 0,             // u8 的默认值是 0
        }
    }
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
            let mut input = Cursor::new(vec![0; 1]); // 创建一个空输入流
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
            let mut input = Cursor::new(vec![1; 1]); // 创建一个空输入流
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
            let mut input = Cursor::new(vec![0; 1]); // 创建一个空输入流
            self.fill_inbuf(&mut input, true, state)?;
            Ok(0) // Placeholder, adjust logic as per the context
        }
    }

    // Function to get the next byte
    pub fn next_byte(&mut self, state: &mut GzipState, w: usize) -> io::Result<u8> {
        self.Get_Byte(state, w)
    }

    // Equivalent to the NEEDBITS macro (requiring more information to be fully accurate)
    pub fn need_bits(&mut self, state: &mut GzipState,k: &mut u32, b: &mut u32, n: u32, w: usize)-> (u32, u32)  {
        while *k < n {
                let byte = match self.next_byte(state, w) {
                    Ok(value) => value, // 解包成功
                    Err(_) => {
                        // 处理错误，例如记录日志或返回默认值
                        return (0, 0);
                    }
                };
                *b |= (u32::from(byte)) << *k;

                *k += 8;
            }
            (*k, *b)
    }

    // Equivalent to DUMPBITS macro
    pub fn dump_bits(&mut self, k: &mut u32, b: &mut u32, n: u32)-> (u32, u32)  {
        let updated_b = *b >> n;
        let updated_k = *k - n;
        (updated_k, updated_b)
    }

    // Function to build the Huffman tree
    pub fn huft_build(
        &mut self,
        b: &[u8],          // 代码长度表 (b 的长度 <= BMAX)
        mut n: usize,           // 代码数目 (n <= N_MAX)
        s: u16,             // 简单值代码的数目 (0..s-1)
        d: &[u16],          // 非简单代码的基值
        e: &[u16],          // 非简单代码的额外位数
        t: &mut Vec<Option<Box<Huft>>>, // 表的结果
        m: &mut u16,        // 最大查找位数
    ) -> u32 {
        let mut c = vec![0; (BMAX + 1).try_into().unwrap()]; // Bit length count table
        let mut v = vec![0; N_MAX.try_into().unwrap()]; // Values in order of bit length
        let mut x = vec![0; (BMAX + 1).try_into().unwrap()]; // Bit offsets, then code stack

        let mut l = *m;
        let mut k = 0; // Minimum code length
        let mut g = 0; // Maximum code length
        let mut w = 0; // Bits decoded (w = l * h)
        let mut h: i32 = -1; // No tables yet—level -1
        let mut y = 0; // Number of dummy codes added
        let mut z = 0; // Number of entries in current table
        let mut a = 0; // Counter for codes of length k

        // Generate counts for each bit length
        for &len in b {
            c[len as usize] += 1;
        }

        if c[0] == n { // Null input—all zero length codes
            let mut q = vec![
                Huft {
                    v: Box::new(None),
                    e: 99,
                    b: 1,
                },
                Huft {
                    v: Box::new(None),
                    e: 99,
                    b: 1,
                },
            ];

            *t = vec![Some(Box::new(q[1].clone()))];
            *m = 1;
            return 0;
        }

        // Find minimum and maximum length, bound *m by those
        for j in 1..=BMAX {
            if c[j as usize] > 0 {
                k = j;
                break;
            }
        }
        if l < k as u16 {
            l = k as u16;
        }

        for i in (1..=BMAX).rev() {
            if c[i as usize] > 0 {
                g = i;
                break;
            }
        }

        if l > g as u16 {
            l = g as u16;
        }
        *m = l;

        // Adjust last length count to fill out codes if needed
        let mut y_val = 1 << k;
        for j in k..g {
            y_val -= c[j as usize];
            if y_val < 0 {
                return 2; // Bad input: more codes than bits
            }
        }
        y_val -= c[g as usize];
        if y_val < 0 {
            return 2;
        }
        c[g as usize] += y_val;

        // Generate starting offsets into the value table for each length
        x[1] = k as u32;
        for i in (2..=g).rev() {
            x[i as usize] = x[(i - 1) as usize] + c[(i - 1) as usize] as u32;
        }

        // Make a table of values in order of bit lengths
        let mut i = 0;
        for &len in b {
            if len != 0 {
                v[x[len as usize] as usize] = i;
                x[len as usize] += 1;
            }
            i += 1;
        }

        n = x[g as usize] as usize; // Set n to length of v


        // Generate the Huffman codes and for each, make the table entries
        x[0] = 0; // First Huffman code is zero
        let mut p = v.iter().cloned();
        h = -1;
        w = -(l as i32); // Bits decoded = (l * h)

        let mut u: Vec<*mut Huft> = vec![null_mut(); BMAX.try_into().unwrap()];
        let mut q = vec![];

        // Go through the bit lengths (k already is bits in shortest code)
        for k in k..=g {
            a = c[k as usize];
            while a > 0 {
                // Make tables up to required level
                while k > w + l as i32 {
                    h += 1;
                    w += l as i32; // Previous table always l bits

                    // Compute minimum size table less than or equal to l bits
                    z = (g - w) as u32;
                    let f = 1 << (k - w);
                    if f > a + 1 {
                        let mut f1 = f - a - 1;
                        let mut j = k - w;
                        while j < z as i32 {
                            if f1 <= c[j as usize] {
                                break;
                            }
                            f1 -= c[j as usize];
                        }
                    }

                    z = 1 << (k - w);
                    if q.len() == 0 {
                        q.push(Huft {
                            v: Box::new(None),
                            e: 0,
                            b: 0,
                        });
                    }
                }

                // Set up table entry in r
                let mut r = Huft {
                    v: Box::new(None),
                    e: 0,
                    b: 0,
                };
                r.b = (k - w) as u8;
                if p.clone().count() >= n {
                    r.e = 99; // Out of values—invalid code
                } else if p.clone().next().unwrap() < s {
                    r.e = if p.clone().next().unwrap() < 256 {
                        16
                    } else {
                        15
                    }; // Simple code
                } else {
                    r.e = e[p.clone().next().unwrap() as usize - s as usize] as u8;
                    r.v = Box::new(Some(Box::new(Huft { v: Box::new(None), e: 0, b: 0 })));
                }

                for i in 0..z as u32 {
                    q[i as usize] = r.clone();
                }

                // Backwards increment the k-bit code i
                let mut j = 1 << (k - 1);
                i ^= j;
                i ^= j;

                while u32::from(i & ((1 << w) - 1)) != x[h as usize] {
                    h -= 1;
                    w -= l as i32;
                }
            }
        }

        return if y != 0 && g != 1 { 1 } else { 0 };
    }

    // Function to inflate coded data
    pub fn inflate_codes(
        &mut self,
        state: &mut GzipState,
        tl: &mut Vec<Option<Box<Huft>>>, // literal/length code table
        td: &mut Vec<Option<Box<Huft>>>, // distance code table
        bl: i32,                    // lookup bits for tl
        bd: i32                     // lookup bits for td
    ) -> i32 {
        let mut e: u32;  // Table entry flag/number of extra bits
        let mut n: u32;  // Length and index for copy
        let mut d: u32;  // Distance for copy
        let mut w: usize = self.wp;  // Current window position
        let mut t: &mut Huft;  // Pointer to table entry
        let mut ml: u32;  // Mask for bl bits
        let mut md: u32;  // Mask for bd bits
        let mut b: u32 = self.bb;  // Bit buffer
        let mut k: u32 = self.bk;  // Number of bits in bit buffer

        // Make local copies of globals
        ml = mask_bits[bl as usize];
        md = mask_bits[bd as usize];

        loop {
            // Simulate NEEDBITS
            self.need_bits(state, &mut k, &mut b, bl as u32, w);

            // Look up the table entry
            e = tl[(b & ml) as usize].clone().unwrap().e as u32;

            if e > 16 {
                while e > 16 {
                    if e == 99 {
                        return 1; // Error code for invalid input
                    }

                    // Simulate DUMPBITS
                    let tl_entry = td[(b & ml) as usize].clone().unwrap();
                    self.dump_bits(&mut k, &mut b, tl_entry.b as u32);
                    e -= 16;

                    // Simulate NEEDBITS
                    self.need_bits(state, &mut k, &mut b, e as u32, w);
                }
            }

            // Simulate DUMPBITS
            let tl_entry = td[(b & ml) as usize].clone().unwrap();
            self.dump_bits(&mut k, &mut b, tl_entry.b as u32);

            if e == 16 { // Literal
                self.slide[w] = tl[(b & ml) as usize].clone().unwrap().v.as_ref().clone().unwrap().e;
                if w == WSIZE {
                    self.flush_output(state, w); // Mock flush output
                    w = 0;
                }
                w += 1;
            } else { // EOB or length
                if e == 15 {
                    break; // End of block
                }

                // Get the length of block to copy
                self.need_bits(state, &mut k, &mut b, e as u32, w);
                n = tl[(b & ml) as usize].clone().unwrap().v.as_ref().clone().unwrap().e as u32 + (b & mask_bits[e as usize]);
                self.dump_bits(&mut k, &mut b, e as u32);

                // Decode the distance of block to copy
                self.need_bits(state, &mut k, &mut b, bd as u32, w);

                e = td[(b & md) as usize].clone().unwrap().e as u32;
                if e > 16 {
                    while e > 16 {
                        if e == 99 {
                            return 1; // Error code for invalid input
                        }
                        let tl_entry = td[(b & md) as usize].clone().unwrap();
                        self.dump_bits(&mut k, &mut b, tl_entry.b as u32);
                        e -= 16;
                        self.need_bits(state, &mut k, &mut b, e as u32, w);
                    }
                }
                // 首先使用 b 计算索引，再将 b 的可变引用传递给 dump_bits
                let tl_entry = td[(b & md) as usize].clone().unwrap(); // 先使用 b 计算索引，得到 tl_entry
                self.dump_bits(&mut k, &mut b, tl_entry.b as u32);

                self.need_bits(state, &mut k, &mut b, e as u32, w);
                d = w as u32 - td[(b & md) as usize].clone().unwrap().v.as_ref().clone().unwrap().e as u32 - (b & mask_bits[e as usize]);
                self.dump_bits(&mut k, &mut b, e as u32);

                // Copy the data
                loop {
                    n -= if n < e { n } else { e };
                    if n == 0 {
                        break;
                    }
                    self.slide[w] = self.slide[d as usize];
                    w += 1;
                    d += 1;
                }
            }
        }

        // Restore globals from locals
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
        let mut i: usize;               // temporary variable
        let mut tl: Vec<Option<Box<Huft>>> = vec![];
        let mut td: Vec<Option<Box<Huft>>> = vec![];
        let mut bl: u16;                 // lookup bits for tl
        let mut bd: u16;                 // lookup bits for td
        let mut l: [u8; 288] = [0; 288]; // length list for huft_build

        // set up literal table
        for i in 0..144 {
            l[i] = 8;
        }
        for i in 144..256 {
            l[i] = 9;
        }
        for i in 256..280 {
            l[i] = 7;
        }
        for i in 280..288 {             // make a complete, but wrong code set
            l[i] = 8;
        }
        bl = 7;

        // Call huft_build for literal/length table
        let i = self.huft_build(&l, 288, 257, &cpdist, &cpdext, &mut tl, &mut bl);
        if i != 0 {
            return i.try_into().unwrap();
        }

        // set up distance table
        for i in 0..30 {                // make an incomplete code set
            l[i] = 5;
        }
        bd = 5;

        // Call huft_build for distance table
        let i = self.huft_build(&l, 30, 0, &cpdist, &cpdext, &mut td, &mut bd);
        if i > 1 {
            for entry in tl {
                huft_free(entry);  // 对每个 Option<Box<Huft>> 调用 huft_free
            }
            return i.try_into().unwrap();
        }

        // decompress until an end-of-block code
        if self.inflate_codes(state, &mut tl, &mut td, bl.into(), bd.into()) != 0 {
            return 1;
        }

        // free the decoding tables, return
        for entry in tl {
            huft_free(entry);  // 对每个 Option<Box<Huft>> 调用 huft_free
        }
        for entry in td {
            huft_free(entry);  // 对每个 Option<Box<Huft>> 调用 huft_free
        }
        return 0;
    }

    // Decompress an inflated type 2 (dynamic Huffman codes) block
    pub fn inflate_dynamic(&mut self, state: &mut GzipState) -> i32 {
        let mut i: u32;                // temporary variables
        let mut j: u32;
        let mut l: u32;                // last length
        let mut m: u32;                // mask for bit lengths table
        let mut n: u32;                // number of lengths to get
        let mut w: u32;                // current window position
        let mut tl: Vec<Option<Box<Huft>>> = vec![];
        let mut td: Vec<Option<Box<Huft>>> = vec![];
        let mut bl: u16;               // lookup bits for tl
        let mut bd: u16;               // lookup bits for td
        let mut nb: u32;               // number of bit length codes
        let mut nl: u32;               // number of literal/length codes
        let mut nd: u32;               // number of distance codes
        let mut ll: Vec<u8> = vec![0; 286 + 30];  // literal/length and distance code lengths
        let mut b: u32;                // bit buffer
        let mut k: u32;                // number of bits in bit buffer

        // make local bit buffer
        b = self.bb;
        k = self.bk;
        w = self.wp as u32;

        // read in table lengths
        self.need_bits(state, &mut k, &mut b, 5, w.try_into().unwrap());
        nl = 257 + (b & 0x1f);      // number of literal/length codes
        self.dump_bits(&mut k, &mut b, 5);
        self.need_bits(state, &mut k, &mut b, 5, w.try_into().unwrap());
        nd = 1 + (b & 0x1f);        // number of distance codes
        self.dump_bits(&mut k, &mut b, 5);
        self.need_bits(state, &mut k, &mut b, 4, w.try_into().unwrap());
        nb = 4 + (b & 0xf);         // number of bit length codes
        self.dump_bits(&mut k, &mut b, 4);

        // Check if the number of codes is valid
        if (nl > 286 || nd > 30) {
            return 1;  // bad lengths
        }

        // read in bit-length-code lengths
        for j in 0..nb {
            self.need_bits(state, &mut k, &mut b, 3, w.try_into().unwrap());
            ll[border[j as usize] as usize] = (b as u8) & 7 ;
            self.dump_bits(&mut k, &mut b, 3);
        }
        for j in nb..19 {
            ll[border[j as usize] as usize] = 0;
        }

        // build decoding table for trees -- single level, 7 bit lookup
        bl = 7;
        i = self.huft_build(&ll, 19, 19, &[], &[], &mut tl, &mut bl);
        if i != 0 {
            if i == 1 {
                for entry in tl {
                    huft_free(entry);  // 对每个 Option<Box<Huft>> 调用 huft_free
                }
            }
            return i.try_into().unwrap();  // incomplete code set
        }

        if tl.is_empty() {
            return 2; // Grrrhhh
        }

        // read in literal and distance code lengths
        n = nl + nd;
        m = mask_bits[bl as usize];
        i = 0;
        l = 0;
        while i < n {
            self.need_bits(state, &mut k, &mut b, bl.into(), w as usize);
            let td = &mut tl[(b & m) as usize].as_mut().unwrap();

            self.dump_bits(&mut k, &mut b, td.b.into());
            if td.e == 99 {
                // Invalid code
                for entry in tl {
                    huft_free(entry);  // 对每个 Option<Box<Huft>> 调用 huft_free
                }
                return 2;
            }
            j = td.v.clone().unwrap().v.as_ref().clone().unwrap().e as u32;
            if j < 16 {              // length of code in bits (0..15)
                l = j;
                ll[i as usize] = l as u8; // save last length in l
            } else if j == 16 {       // repeat last length 3 to 6 times
                self.need_bits(state, &mut k, &mut b, 2, w.try_into().unwrap());
                j = 3 + (b & 3);
                self.dump_bits(&mut k, &mut b, 2);
                if i + j > n {
                    return 1;
                }
                while j > 0 {
                    ll[i as usize] = l as u8;
                    i += 1;
                    j -= 1;
                }
            } else if j == 17 {       // 3 to 10 zero length codes
                self.need_bits(state, &mut k, &mut b, 3, w.try_into().unwrap());
                j = 3 + (b & 7);
                self.dump_bits(&mut k, &mut b, 3);
                if i + j > n {
                    return 1;
                }
                while j > 0 {
                    ll[i as usize] = 0;
                    i += 1;
                    j -= 1;
                }
                l = 0;
            } else {                  // j == 18: 11 to 138 zero length codes
                self.need_bits(state, &mut k, &mut b, 7, w.try_into().unwrap());
                j = 11 + (b & 0x7f);
                self.dump_bits(&mut k, &mut b, 7);
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

        // free decoding table for trees
        for entry in &tl {
            huft_free(entry.clone());  // 对每个 Option<Box<Huft>> 调用 huft_free
        }

        // restore the global bit buffer
        self.bb = b;
        self.bk = k;

        // build the decoding tables for literal/length and distance codes
        bl = self.lbits as u16;
        i = self.huft_build(&ll, nl.try_into().unwrap(), 257, &cpdist, &cpdext, &mut tl, &mut bl);
        if i != 0 {
            if i == 1 {
                eprintln!("incomplete literal tree");
                for entry in tl {
                    huft_free(entry);  // 对每个 Option<Box<Huft>> 调用 huft_free
                }
            }
            return i.try_into().unwrap();  // incomplete code set
        }
        bd = self.dbits as u16;
        i = self.huft_build(&ll[nl as usize..], nd.try_into().unwrap(), 0, &cpdist, &cpdext, &mut td, &mut bd);
        if i != 0 {
            if i == 1 {
                eprintln!("incomplete distance tree");
                for entry in td {
                    huft_free(entry);  // 对每个 Option<Box<Huft>> 调用 huft_free
                }
            }
            for entry in tl {
                huft_free(entry);  // 对每个 Option<Box<Huft>> 调用 huft_free
            }
            return i.try_into().unwrap();  // incomplete code set
        }

        // decompress until an end-of-block code
        let err = if self.inflate_codes(state, &mut tl, &mut td, bl.into(), bd.into())>0 { 1 } else { 0 };


        // free the decoding tables
        for entry in tl {
            huft_free(entry);  // 对每个 Option<Box<Huft>> 调用 huft_free
        }
        for entry in td {
            huft_free(entry);  // 对每个 Option<Box<Huft>> 调用 huft_free
        }

        err

    }


    // Decompress an inflated block
    // E is the last block flag
    pub fn inflate_block(&mut self, e: &mut i32) -> i32 {
        unimplemented!()
    }


    // Decompress an inflated entry
    pub fn inflate(&mut self, state: &mut GzipState) -> i32 {
        unimplemented!()
    }
}













