use crate::deflate::{Deflate, MAX_DIST, MAX_MATCH, MIN_MATCH};
use crate::{GzipState, STORED};
use std::cell::RefCell;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;

const MAX_BITS: usize = 15;
const MAX_BL_BITS: usize = 7;
const LENGTH_CODES: usize = 29;
const LITERALS: usize = 256;
const LIT_BUFSIZE: usize = 0x8000;
const DIST_BUFSIZE: usize = 0x8000;
const L_CODES: usize = LITERALS + 1 + LENGTH_CODES;
const D_CODES: usize = 30;
const BL_CODES: usize = 19;
const HEAP_SIZE: usize = 2 * L_CODES + 1;
const END_BLOCK: usize = 256;
const STORED_BLOCK: usize = 0;
const STATIC_TREES: usize = 1;
const DYN_TREES: usize = 2;
const SMALLEST: usize = 1;
const BINARY: u16 = 0;
const ASCII: u16 = 1;
const REP_3_6: usize = 16;
/* repeat previous bit length 3-6 times (2 bits of repeat count) */

const REPZ_3_10: usize = 17;
/* repeat a zero length 3-10 times  (3 bits of repeat count) */

const REPZ_11_138: usize = 18;
/* repeat a zero length 11-138 times  (7 bits of repeat count) */

const EXTRA_LBITS: [i32; LENGTH_CODES] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1,
    2, 2, 2, 2, 3, 3, 3, 3,
    4, 4, 4, 4, 5, 5, 5, 5, 0,
];

const EXTRA_DBITS: [i32; D_CODES] = [
    0, 0, 0, 0, 1, 1, 2, 2,
    3, 3, 4, 4, 5, 5, 6, 6,
    7, 7, 8, 8, 9, 9, 10, 10,
    11, 11, 12, 12, 13, 13,
];

const EXTRA_BLBITS: [i32; BL_CODES] = [
    0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0,
    0, 0, 2, 3, 7
];

const BL_ORDER: [usize; BL_CODES] = [
    16, 17, 18, 0, 8, 7, 9, 6,
    10, 5, 11, 4, 12, 3, 13, 2,
    14, 1, 15,
];

#[derive(Default, Copy, Clone)]
struct CtData {
    freq: u16,
    len: u16,
    code: u16,
    dad: u16
}

pub struct Trees<'a> {
    file_type: Option<&'a mut u16>,
    file_method: i32,
    compressed_len: u64,
    input_len: u64,
    base_length: [i32; LENGTH_CODES],
    base_dist: [i32; D_CODES],
    length_code: [u8; 256],
    dist_code: [u8; 512],
    bl_count: [i32; MAX_BITS + 1],
    static_ltree: Rc<RefCell<Vec<CtData>>>,
    static_dtree: Rc<RefCell<Vec<CtData>>>,
    bltree: Rc<RefCell<Vec<CtData>>>,
    dyn_ltree: Rc<RefCell<Vec<CtData>>>,
    dyn_dtree: Rc<RefCell<Vec<CtData>>>,
    bl_tree: Box<[CtData; 2 * BL_CODES + 1]>,
    opt_len: u64,
    static_len: u64,
    last_lit: i32,
    last_dist: i32,
    last_flags: i32,
    flags: i32,
    flag_bit: i32,
    l_buf: Box<[usize; LIT_BUFSIZE]>,
    d_buf: Box<[usize; DIST_BUFSIZE]>,
    flag_buf: Box<[usize; LIT_BUFSIZE/8]>,
    l_desc: TreeDesc<'a>,
    d_desc: TreeDesc<'a>,
    bl_desc: TreeDesc<'a>,
    heap: [i32; 2*L_CODES+1],
    depth: [i32; 2*L_CODES+1],
    heap_len: usize,
    heap_max: usize
}

#[derive(Clone)]
struct TreeDesc<'a> {
    dyn_tree: Rc<RefCell<Vec<CtData>>>,      // The dynamic tree
    static_tree: Option<Rc<RefCell<Vec<CtData>>>>, // The corresponding static tree or None
    extra_bits: Option<&'a [i32]>,    // Extra bits for each code or None
    extra_base: usize,
    elems: usize,                    // Number of elements in the tree
    max_length: usize,               // Maximum bit length for the codes
    max_code: i32,                   // Largest code with non-zero frequency
}

impl<'a> Trees<'a> {
    pub fn new() -> Self {
        let static_ltree = Rc::new(RefCell::new(vec![CtData::default(); L_CODES + 2]));
        let static_dtree = Rc::new(RefCell::new(vec![CtData::default(); D_CODES]));
        let bltree = Rc::new(RefCell::new(vec![CtData::default(); 2 * BL_CODES + 1]));
        let dyn_ltree = Rc::new(RefCell::new(vec![CtData::default(); HEAP_SIZE]));
        let dyn_dtree = Rc::new(RefCell::new(vec![CtData::default(); 2 * D_CODES + 1]));
        Self {
            file_type: None,
            file_method: 0,
            compressed_len: 0,
            input_len: 0,
            base_length: [0; LENGTH_CODES],
            base_dist: [0; D_CODES],
            length_code: [0; 256],
            dist_code: [0; 512],
            bl_count: [0; MAX_BITS + 1],
            static_ltree: static_ltree.clone(),
            static_dtree: static_dtree.clone(),
            bltree: bltree.clone(),
            dyn_ltree: dyn_ltree.clone(),
            dyn_dtree: dyn_dtree.clone(),
            bl_tree: Box::new([CtData::default(); 2 * BL_CODES + 1]),
            opt_len: 0,
            static_len: 0,
            last_lit: 0,
            last_dist: 0,
            last_flags: 0,
            flags: 0,
            flag_bit: 1,
            l_buf: Box::new([0; LIT_BUFSIZE]),
            d_buf: Box::new([0; DIST_BUFSIZE]),
            flag_buf: Box::new([0; LIT_BUFSIZE/8]),
            l_desc: TreeDesc {
                dyn_tree: dyn_ltree,
                static_tree: Some(static_ltree),
                extra_bits: Some(&EXTRA_LBITS),
                extra_base: LITERALS+1,
                elems: L_CODES,
                max_length: MAX_BITS,
                max_code: 0,
            },
            d_desc: TreeDesc {
                dyn_tree: dyn_dtree,
                static_tree: Some(static_dtree),
                extra_bits: Some(&EXTRA_DBITS),
                extra_base: 0,
                elems: D_CODES,
                max_length: MAX_BITS,
                max_code: 0,
            },
            bl_desc: TreeDesc {
                dyn_tree: bltree,
                static_tree: None,
                extra_bits: Some(&EXTRA_BLBITS),
                extra_base: 0,
                elems: BL_CODES,
                max_length: MAX_BL_BITS,
                max_code: 0,
            },
            heap: [0; 2*L_CODES+1],
            depth: [0; 2*L_CODES+1],
            heap_len: 0,
            heap_max: 0
        }
    }

    pub(crate) fn ct_init(&mut self, attr: &'a mut u16, methodp: i32) {
        let mut n: i32;
        let mut length: i32;
        let mut code: i32;
        let mut dist: i32;

        self.file_type = Some(attr);
        self.file_method = methodp;
        self.compressed_len = 0;
        self.input_len = 0;

        if self.static_dtree.borrow()[0].len != 0 {
            return; // ct_init already called
        }

        // Initialize the mapping length (0..255) -> length code (0..28)
        length = 0;
        code = 0;
        while code < (LENGTH_CODES - 1) as i32 {
            self.base_length[code as usize] = length;
            n = 0;
            while n < (1 << EXTRA_LBITS[code as usize]) {
                self.length_code[length as usize] = code as u8;
                length += 1;
                n += 1;
            }
            code += 1;
        }
        assert!(length == 256, "ct_init: length != 256");

        // Overwrite length_code[255] to use the best encoding
        self.length_code[(length - 1) as usize] = code as u8;

        // Initialize the mapping dist (0..32K) -> dist code (0..29)
        dist = 0;
        code = 0;
        while code < 16 {
            self.base_dist[code as usize] = dist;
            n = 0;
            while n < (1 << EXTRA_DBITS[code as usize]) {
                self.dist_code[dist as usize] = code as u8;
                dist += 1;
                n += 1;
            }
            code += 1;
        }
        assert!(dist == 256, "ct_init: dist != 256");

        dist >>= 7; // From now on, all distances are divided by 128
        while code < D_CODES as i32 {
            self.base_dist[code as usize] = dist << 7;
            n = 0;
            while n < (1 << (EXTRA_DBITS[code as usize] - 7)) {
                self.dist_code[(256 + dist) as usize] = code as u8;
                dist += 1;
                n += 1;
            }
            code += 1;
        }
        assert!(dist == 256, "ct_init: 256+dist != 512");

        // Construct the codes of the static literal tree
        for bits in 0..=MAX_BITS as i32 {
            self.bl_count[bits as usize] = 0;
        }

        n = 0;
        while n <= 143 {
            self.static_ltree.borrow_mut()[n as usize].len = 8;
            self.bl_count[8] += 1;
            n += 1;
        }
        while n <= 255 {
            self.static_ltree.borrow_mut()[n as usize].len = 9;
            self.bl_count[9] += 1;
            n += 1;
        }
        while n <= 279 {
            self.static_ltree.borrow_mut()[n as usize].len = 7;
            self.bl_count[7] += 1;
            n += 1;
        }
        while n <= 287 {
            self.static_ltree.borrow_mut()[n as usize].len = 8;
            self.bl_count[8] += 1;
            n += 1;
        }

        // Generate the codes
        Self::gen_codes(&self.bl_count, self.static_ltree.borrow_mut().deref_mut(), (L_CODES + 1) as i32);

        // The static distance tree is trivial
        for n in 0..D_CODES as i32 {
            self.static_dtree.borrow_mut()[n as usize].len = 5;
            self.static_dtree.borrow_mut()[n as usize].code = Self::bi_reverse(n as u16, 5);
        }

        // Initialize the first block of the first file
        self.init_block();
    }

    fn init_block(&mut self) {
        // Initialize the dynamic literal tree frequencies
        for n in 0..L_CODES {
            self.dyn_ltree.borrow_mut()[n].freq = 0;
        }

        // Initialize the dynamic distance tree frequencies
        for n in 0..D_CODES {
            self.dyn_dtree.borrow_mut()[n].freq = 0;
        }

        // Initialize the bit length tree frequencies
        for n in 0..BL_CODES {
            self.bl_tree[n].freq = 0;
        }

        // Set the frequency of the END_BLOCK symbol to 1
        self.dyn_ltree.borrow_mut()[END_BLOCK].freq = 1;
    }

    fn gen_codes(bl_count: &[i32; MAX_BITS + 1], tree: &mut [CtData], max_code: i32) {
        let mut next_code = [0u16; MAX_BITS + 1];
        let mut code = 0u16;

        // Generate the next_code array
        for bits in 1..=MAX_BITS {
            code = ((code as i32 + bl_count[bits - 1]) << 1) as u16;
            next_code[bits] = code;
        }

        // Assign codes to tree nodes
        for n in 0..max_code as usize {
            let len = tree[n].len as usize;
            if len != 0 {
                tree[n].code = Self::bi_reverse(next_code[len], len);
                next_code[len] += 1;
            }
        }
    }

    fn bi_reverse(code: u16, len: usize) -> u16 {
        let mut code = code;
        let mut res = 0u16;
        for _ in 0..len {
            res = (res << 1) | (code & 1);
            code >>= 1;
        }
        res
    }

    pub fn ct_tally(&mut self, deflate: &mut Deflate, state: &mut GzipState, dist: usize, lc: usize) -> bool {
        // Add the character or match length to the literal buffer
        self.l_buf[self.last_lit as usize] = lc as u8 as usize;
        self.last_lit += 1;

        if dist == 0 {
            // lc is the unmatched character (literal)
            self.dyn_ltree.borrow_mut()[lc].freq += 1;
        } else {
            // lc is the match length - MIN_MATCH
            let dist = dist - 1; // Adjust distance
            assert!(
                dist < MAX_DIST
                    && lc <= MAX_MATCH - MIN_MATCH
                    && self.d_code(dist) < D_CODES,
                "ct_tally: bad match"
            );

            self.dyn_ltree.borrow_mut()[self.length_code[lc] as usize + LITERALS + 1].freq += 1;
            self.dyn_dtree.borrow_mut()[self.d_code(dist)].freq += 1;

            self.d_buf[self.last_dist as usize] = dist as u16 as usize;
            self.last_dist += 1;
            self.flags |= self.flag_bit;
        }

        self.flag_bit <<= 1;

        // Output the flags if they fill a byte
        if (self.last_lit & 7) == 0 {
            self.flag_buf[self.last_flags as usize] = self.flags as usize;
            self.last_flags += 1;
            self.flags = 0;
            self.flag_bit = 1;
        }

        // Try to guess if it is profitable to stop the current block here
        if state.level > 2 && (self.last_lit & 0xfff) == 0 {
            // Compute an upper bound for the compressed length
            let mut out_length = self.last_lit as u64 * 8;
            let in_length = deflate.strstart - deflate.block_start as usize;

            for dcode in 0..D_CODES {
                out_length += self.dyn_dtree.borrow()[dcode].freq as u64
                    * (5 + EXTRA_DBITS[dcode] as u64);
            }

            out_length >>= 3; // Divide by 8

            if state.verbose > 0 {
                eprintln!(
                    "\nlast_lit {}, last_dist {}, in {}, out ~{}({}%)",
                    self.last_lit,
                    self.last_dist,
                    in_length,
                    out_length,
                    100 - out_length * 100 / in_length as u64
                );
            }

            if self.last_dist < self.last_lit / 2 && out_length < (in_length / 2) as u64 {
                return true;
            }
        }

        // Return true if the buffer is full
        self.last_lit == (LIT_BUFSIZE - 1) as i32 || self.last_dist == DIST_BUFSIZE as i32
    }

    fn d_code(&self, dist: usize) -> usize {
        if dist < 256 {
            self.dist_code[dist] as usize
        } else {
            self.dist_code[256 + (dist >> 7)] as usize
        }
    }

    pub(crate) fn flush_block(
        &mut self,
        state: &mut GzipState,
        buf: Option<&[u8]>,
        stored_len: u64,
        eof: bool,
    ) -> i64 {
        let mut opt_lenb: u64;
        let static_lenb: u64;
        let max_blindex: i32;

        // Save the flags for the last 8 items
        self.flag_buf[self.last_flags as usize] = self.flags as usize;

        // Check if the file is ASCII or binary
        if self.file_type == None {
            self.set_file_type();
        }

        // Construct the literal and distance trees
        self.build_tree(state, &mut self.l_desc.clone());
        if state.verbose > 1 {
            eprintln!(
                "\nlit data: dyn {}, stat {}",
                self.opt_len, self.static_len
            );
        }

        self.build_tree(state, &mut self.d_desc.clone());
        if state.verbose > 1 {
            eprintln!(
                "\ndist data: dyn {}, stat {}",
                self.opt_len, self.static_len
            );
        }

        // Build the bit length tree and get the index of the last bit length code to send
        max_blindex = self.build_bl_tree(state);

        // Determine the best encoding. Compute the block length in bytes
        opt_lenb = (self.opt_len + 3 + 7) >> 3;
        static_lenb = (self.static_len.wrapping_add(3 + 7)) >> 3;
        self.input_len += stored_len; // For debugging only

        if state.verbose > 0 {
            eprintln!(
                "\nopt {}({}) stat {}({}) stored {} lit {} dist {}",
                opt_lenb,
                self.opt_len,
                static_lenb,
                self.static_len,
                stored_len,
                self.last_lit,
                self.last_dist
            );
        }

        if static_lenb <= opt_lenb {
            opt_lenb = static_lenb;
        }

        if stored_len <= opt_lenb && eof && self.compressed_len == 0 {
            // Since LIT_BUFSIZE <= 2*WSIZE, the input data must be there
            if buf.is_none() {
                state.gzip_error("block vanished");
            }

            self.copy_block(state, buf.unwrap(), stored_len as usize, false); // Without header
            self.compressed_len = stored_len << 3;
            self.file_method = STORED as i32;
        } else if stored_len + 4 <= opt_lenb && buf.is_some() {
            // 4: two words for the lengths
            let eof_flag = if eof { 1 } else { 0 };
            state.send_bits(((STORED_BLOCK << 1) + eof_flag) as u16, 3); // Send block type
            self.compressed_len = (self.compressed_len + 3 + 7) & !7u64;
            self.compressed_len += (stored_len + 4) << 3;

            self.copy_block(state, buf.unwrap(), stored_len as usize, true); // With header
        } else if static_lenb == opt_lenb {
            let eof_flag = if eof { 1 } else { 0 };
            state.send_bits(((STATIC_TREES << 1) + eof_flag) as u16, 3);
            self.compress_block(state, self.static_ltree.clone().borrow().deref(), self.static_dtree.clone().borrow().deref());
            self.compressed_len += 3 + self.static_len;
        } else {
            let eof_flag = if eof { 1 } else { 0 };
            state.send_bits(((DYN_TREES << 1) + eof_flag) as u16, 3);
            self.send_all_trees(
                state,
                (self.l_desc.max_code + 1) as usize,
                (self.d_desc.max_code + 1) as usize,
                (max_blindex + 1) as usize,
            );
            self.compress_block(state, self.dyn_ltree.clone().borrow().deref(), self.dyn_dtree.clone().borrow().deref());
            self.compressed_len += 3 + self.opt_len;
        }

        self.init_block();

        if eof {
            //assert!(self.input_len as i64 == state.bytes_in, "bad input size");
            state.bi_windup();
            self.compressed_len = self.compressed_len.wrapping_add(7); // Align on byte boundary
        }

        (self.compressed_len >> 3) as i64
    }

    /// Send the header for a block using dynamic Huffman trees:
    /// the counts, the lengths of the bit length codes, the literal tree, and the distance tree.
    /// IN assertion: lcodes >= 257, dcodes >= 1, blcodes >= 4.
    fn send_all_trees(&mut self, state: &mut GzipState, lcodes: usize, dcodes: usize, blcodes: usize) {
        // Assertions to ensure we have the correct number of codes
        assert!(
            lcodes >= 257 && dcodes >= 1 && blcodes >= 4,
            "not enough codes"
        );
        assert!(
            lcodes <= L_CODES && dcodes <= D_CODES && blcodes <= BL_CODES,
            "too many codes"
        );

        // Optional debugging output
        if state.verbose > 1 {
            eprintln!("\nbl counts:");
        }

        // Send the number of literal codes, distance codes, and bit length codes
        state.send_bits((lcodes - 257) as u16, 5); // lcodes - 257 in 5 bits
        state.send_bits((dcodes - 1) as u16, 5);   // dcodes - 1 in 5 bits
        state.send_bits((blcodes - 4) as u16, 4);  // blcodes - 4 in 4 bits

        // Send the bit length codes in the order specified by bl_order
        for rank in 0..blcodes {
            let bl_code = BL_ORDER[rank];

            if state.verbose > 1 {
                eprintln!("\nbl code {:2}", bl_code);
            }

            // Send the bit length for the current code in 3 bits
            state.send_bits(self.bl_tree[bl_code].len as u16, 3);
        }

        // Send the literal tree
        self.send_tree(state, &self.dyn_ltree.clone().borrow().deref(), lcodes - 1);

        // Send the distance tree
        self.send_tree(state, self.dyn_dtree.clone().borrow().deref(), dcodes - 1);
    }

    /// Send a literal or distance tree in compressed form, using the codes in bl_tree.
    fn send_tree(&mut self, state: &mut GzipState, tree: &[CtData], max_code: usize) {
        let mut prevlen: i32 = -1; // Last emitted length
        let mut curlen: i32; // Length of current code
        let mut nextlen: i32 = tree[0].len as i32; // Length of next code
        let mut count: i32 = 0; // Repeat count of the current code length
        let mut max_count: i32 = 7; // Max repeat count
        let mut min_count: i32 = 4; // Min repeat count

        // If the first code length is zero, adjust max and min counts
        if nextlen == 0 {
            max_count = 138;
            min_count = 3;
        }

        for n in 0..=max_code {
            curlen = nextlen;
            if n + 1 <= max_code {
                nextlen = tree[n + 1].len as i32;
            } else {
                nextlen = -1;
            }

            count += 1;

            if count < max_count && curlen == nextlen {
                continue;
            } else {
                if count < min_count {
                    // Send the code 'count' times
                    for _ in 0..count {
                        self.send_code(state, curlen as usize, self.bl_tree.deref());
                    }
                } else if curlen != 0 {
                    if curlen != prevlen {
                        self.send_code(state, curlen as usize, self.bl_tree.deref());
                        count -= 1;
                    }
                    assert!(
                        count >= 3 && count <= 6,
                        "Invalid count for REP_3_6: count = {}",
                        count
                    );
                    self.send_code(state, REP_3_6, self.bl_tree.deref());
                    state.send_bits((count - 3) as u16, 2);
                } else if count <= 10 {
                    self.send_code(state, REPZ_3_10, self.bl_tree.deref());
                    state.send_bits((count - 3) as u16, 3);
                } else {
                    self.send_code(state, REPZ_11_138, self.bl_tree.deref());
                    state.send_bits((count - 11) as u16, 7);
                }

                count = 0;
                prevlen = curlen;

                if nextlen == 0 {
                    max_count = 138;
                    min_count = 3;
                } else if curlen == nextlen {
                    max_count = 6;
                    min_count = 3;
                } else {
                    max_count = 7;
                    min_count = 4;
                }
            }
        }
    }

    fn set_file_type(&mut self) {
        let mut n = 0;
        let mut ascii_freq: u32 = 0;
        let mut bin_freq: u32 = 0;

        while n < 7 {
            bin_freq += self.dyn_ltree.borrow()[n].freq as u32;
            n += 1;
        }
        while n < 128 {
            ascii_freq += self.dyn_ltree.borrow()[n].freq as u32;
            n += 1;
        }
        while n < LITERALS {
            bin_freq += self.dyn_ltree.borrow()[n].freq as u32;
            n += 1;
        }

        **self.file_type.as_mut().unwrap() = if bin_freq > (ascii_freq >> 2) {
            BINARY
        } else {
            ASCII
        };
    }

    fn warning(&self, msg: &str) {
        eprintln!("Warning: {}", msg);
    }


    /// Send the block data compressed using the given Huffman trees
    fn compress_block(&mut self, state: &mut GzipState, ltree: &[CtData], dtree: &[CtData]) {
        let mut dist: u32;      // Distance of matched string
        let mut lc: i32;        // Match length or unmatched char (if dist == 0)
        let mut lx: usize = 0;  // Running index in l_buf
        let mut dx: usize = 0;  // Running index in d_buf
        let mut fx: usize = 0;  // Running index in flag_buf
        let mut flag: u8 = 0;   // Current flags
        let mut code: usize;    // The code to send
        let mut extra: u8;      // Number of extra bits to send

        // Check if there are any literals to process
        if self.last_lit != 0 {
            while lx < self.last_lit as usize {
                // Load a new flag byte every 8 literals
                if (lx & 7) == 0 {
                    flag = self.flag_buf[fx] as u8;
                    fx += 1;
                }

                lc = self.l_buf[lx] as i32;
                lx += 1;

                if (flag & 1) == 0 {
                    // It's a literal byte
                    self.send_code(state, lc as usize, ltree); // Send a literal byte
                    // Optionally trace the literal character
                    // if lc is printable, you can log it for debugging
                } else {
                    // It's a match
                    // Here, lc is the match length minus MIN_MATCH
                    let lc_usize = lc as usize;
                    code = self.length_code[lc_usize] as usize;
                    self.send_code(state, code + LITERALS + 1, ltree); // Send the length code
                    extra = EXTRA_LBITS[code] as u8;

                    if extra != 0 {
                        let base_len = self.base_length[code] as i32;
                        let lc_adjusted = lc - base_len;
                        state.send_bits(lc_adjusted as u16, extra); // Send the extra length bits
                    }

                    dist = self.d_buf[dx] as u32;
                    dx += 1;

                    // Here, dist is the match distance minus 1
                    code = self.d_code(dist as usize);
                    assert!(code < D_CODES, "bad d_code");

                    self.send_code(state, code, dtree); // Send the distance code
                    extra = EXTRA_DBITS[code] as u8;

                    if extra != 0 {
                        let base_dist = self.base_dist[code] as u32;
                        let dist_adjusted = dist - base_dist;
                        state.send_bits(dist_adjusted as u16, extra); // Send the extra distance bits
                    }
                }

                flag >>= 1; // Move to the next flag bit
            }
        }

        // Send the end of block code
        self.send_code(state, END_BLOCK, ltree);
    }

    fn send_code(&self, state: &mut GzipState, c: usize, tree: &[CtData]) {
        // Debugging output if verbose > 1
        if state.verbose > 1 {
            eprintln!("\ncd {:3}", c);
        }

        // Send the code and its length using the send_bits function
        state.send_bits(tree[c].code, tree[c].len as u8);
    }

    fn copy_block(&mut self, state: &mut GzipState, buf: &[u8], len: usize, header: bool) {
        // Align on byte boundary
        state.bi_windup();

        if header {
            state.put_short(len as u16);
            state.put_short(!len as u16);
        }

        // Iterate over the buffer and output each byte
        // If encryption is needed, handle it here
        for &byte in buf.iter().take(len) {
            #[cfg(feature = "encryption")]
            {
                // Placeholder for encryption logic
                let encrypted_byte = if self.key.is_some() {
                    self.zencode(byte)
                } else {
                    byte
                };
                self.put_byte(encrypted_byte);
            }
            #[cfg(not(feature = "encryption"))]
            {
                state.put_byte(byte).expect("Failed");
            }
        }
    }

    fn build_tree(&mut self, state: &GzipState, desc: &mut TreeDesc) {
        let tree = desc.dyn_tree.clone();
        let stree = desc.static_tree.as_ref();
        let elems = desc.elems;
        let mut n: usize;
        let mut m: usize;
        let mut max_code = -1; // Largest code with non-zero frequency
        let mut node = elems;  // Next internal node of the tree

        // Construct the initial heap, with the least frequent element at heap[SMALLEST].
        // The sons of heap[n] are heap[2*n] and heap[2*n+1]. heap[0] is not used.
        self.heap_len = 0;
        self.heap_max = HEAP_SIZE;

        for n in 0..elems {
            if tree.borrow()[n].freq != 0 {
                self.heap_len += 1;
                self.heap[self.heap_len] = n as i32;
                max_code = n as i32;
                self.depth[n] = 0;
            } else {
                tree.borrow_mut()[n].len = 0;
            }
        }

        // The PKZIP format requires that at least one distance code exists,
        // and that at least one bit should be sent even if there is only one
        // possible code. So to avoid special checks later on, we force at least
        // two codes of non-zero frequency.
        while self.heap_len < 2 {
            max_code += 1;
            let new_node = if max_code < 2 { max_code } else { 0 } as usize;
            self.heap_len += 1;
            self.heap[self.heap_len] = new_node as i32;
            tree.borrow_mut()[new_node].freq = 1;
            self.depth[new_node] = 0;
            self.opt_len = self.opt_len.wrapping_sub(1);
            if let Some(stree) = stree {
                self.static_len = self.static_len.wrapping_sub(stree.borrow()[new_node].len as u64);
            }
            // new_node is 0 or 1, so it does not have extra bits
        }
        desc.max_code = max_code as i32;

        // The elements heap[heap_len/2+1 .. heap_len] are leaves of the tree,
        // establish sub-heaps of increasing lengths:
        for n in (1..=(self.heap_len / 2)).rev() {
            self.pq_down_heap(tree.borrow().deref(), n);
        }

        // Construct the Huffman tree by repeatedly combining the two least frequent nodes.
        loop {
            n = self.pq_remove(tree.borrow().deref()) as usize; // Node of least frequency
            m = self.heap[SMALLEST] as usize;  // Node of next least frequency

            self.heap_max -= 1;
            self.heap[self.heap_max] = n as i32; // Keep the nodes sorted by frequency
            self.heap_max -= 1;
            self.heap[self.heap_max] = m as i32;

            // Create a new node as the parent of n and m
            let freq = tree.borrow()[n].freq + tree.borrow()[m].freq;
            tree.borrow_mut()[node].freq = freq;
            self.depth[node] = self.depth[n].max(self.depth[m]) + 1;
            tree.borrow_mut()[n].dad = node as u16;
            tree.borrow_mut()[m].dad = node as u16;

            // Insert the new node into the heap
            self.heap[SMALLEST] = node as i32;
            self.pq_down_heap(tree.borrow_mut().deref_mut(), SMALLEST);

            node += 1;
            if self.heap_len < 2 {
                break;
            }
        }

        self.heap_max -= 1;
        self.heap[self.heap_max] = self.heap[SMALLEST];

        // At this point, the fields freq and dad are set. We can now generate the bit lengths.
        self.gen_bitlen(state, desc);

        // The field len is now set; we can generate the bit codes
        Self::gen_codes(&self.bl_count, tree.borrow_mut().deref_mut(), desc.max_code);
    }

    /// Remove the smallest element from the heap and adjust the heap.
    /// Returns the index of the smallest node.
    fn pq_remove(&mut self, tree: &[CtData]) -> usize {
        // The smallest item is at the root of the heap (index 0 in zero-based indexing)
        let top = self.heap[SMALLEST]; // Remove the smallest item

        // Move the last item to the root and reduce the heap size
        self.heap[SMALLEST] = self.heap[self.heap_len - 1];
        self.heap_len -= 1;

        // Restore the heap property by moving down from the root
        self.pq_down_heap(tree, SMALLEST);

        top as usize // Return the index of the smallest node
    }

    /// Compute the optimal bit lengths for a tree and update the total bit length
    /// for the current block.
    /// IN assertion: the fields freq and dad are set, heap[heap_max] and
    /// above are the tree nodes sorted by increasing frequency.
    /// OUT assertions: the field len is set to the optimal bit length, the
    /// array bl_count contains the frequencies for each bit length.
    /// The length opt_len is updated; static_len is also updated if stree is
    /// not null.
    fn gen_bitlen(&mut self, state: &GzipState, desc: &mut TreeDesc) {
        let tree = &mut desc.dyn_tree; // Dynamic tree
        let extra = desc.extra_bits;   // Extra bits array
        let base = desc.extra_base;    // Base index for extra bits
        let max_code = desc.max_code;  // Maximum code with non-zero frequency
        let max_length = desc.max_length; // Maximum allowed bit length
        let stree = &desc.static_tree;  // Static tree (if any)

        let mut overflow = 0; // Number of elements with bit length too large

        // Initialize bl_count array to zero
        for bits in 0..=MAX_BITS {
            self.bl_count[bits as usize] = 0;
        }

        // In a first pass, compute the optimal bit lengths (which may overflow)
        tree.borrow_mut()[self.heap[self.heap_max as usize] as usize].len = 0; // Root of the heap

        for h in (self.heap_max + 1)..self.heap_len {
            let n = self.heap[h as usize] as usize;
            let mut bits = tree.borrow()[tree.borrow()[n].dad as usize].len + 1;

            if bits > max_length as u16 {
                bits = max_length as u16;
                overflow += 1;
            }

            tree.borrow_mut()[n].len = bits;

            // If it's not a leaf node, continue
            if n > max_code as usize {
                continue;
            }

            // Count the frequencies for each bit length
            self.bl_count[bits as usize] += 1;

            let mut xbits = 0;
            if n >= base {
                xbits = extra.as_ref().unwrap()[n - base];
            }

            let f = tree.borrow()[n].freq as u64;
            self.opt_len += f * (bits as u64 + xbits as u64);
            if let Some(stree) = stree {
                self.static_len += f * (stree.borrow()[n].len as u64 + xbits as u64);
            }
        }

        if overflow == 0 {
            return;
        }

        // Adjust bit lengths to eliminate overflow
        self.adjust_bit_lengths(state, overflow, max_length as i32);

        // Now recompute all bit lengths, scanning in increasing frequency
        self.recompute_bit_lengths(state, tree.borrow_mut().deref_mut(), max_code, max_length as i32);
    }

    /// Adjust bit lengths to eliminate overflow
    fn adjust_bit_lengths(&mut self, state: &GzipState, mut overflow: i32, max_length: i32) {
        // This happens for example on obj2 and pic of the Calgary corpus
        if state.verbose > 0 {
            eprintln!("\nbit length overflow");
        }

        // Find the first bit length which could increase
        loop {
            let mut bits = max_length - 1;
            while self.bl_count[bits as usize] == 0 {
                bits -= 1;
            }

            // Decrease count of bit length `bits`
            self.bl_count[bits as usize] -= 1;

            // Increase count of bit length `bits + 1` by 2
            self.bl_count[(bits + 1) as usize] += 2;

            // Decrease count of bit length `max_length`
            self.bl_count[max_length as usize] -= 1;

            overflow -= 2;

            if overflow <= 0 {
                break;
            }
        }
    }

    /// Recompute all bit lengths, scanning in increasing frequency
    fn recompute_bit_lengths(&mut self, state: &GzipState, tree: &mut [CtData], max_code: i32, max_length: i32) {
        let mut h = self.heap_len as usize;
        // Start from the largest bit length
        for bits in (1..=max_length).rev() {
            let n = self.bl_count[bits as usize];
            for _ in 0..n {
                h -= 1;
                let m = self.heap[h] as usize;

                if m > max_code as usize {
                    continue;
                }

                if tree[m].len != bits as u16 {
                    if state.verbose > 1 {
                        eprintln!(
                            "code {} bits {}->{}",
                            m,
                            tree[m].len,
                            bits
                        );
                    }
                    let freq = tree[m].freq as u64;
                    self.opt_len += (bits as u64 - tree[m].len as u64) * freq;
                    tree[m].len = bits as u16;
                }
            }
        }
    }

    /// Restore the heap property by moving down the tree starting at node `k`,
    /// exchanging a node with the smallest of its two children if necessary,
    /// stopping when the heap property is re-established (each parent smaller than its two children).
    fn pq_down_heap(&mut self, tree: &[CtData], mut k: usize) {
        let heap_len = self.heap_len;

        let v = self.heap[k];

        loop {
            let mut j = 2 * k + 1; // Left child index in zero-based array

            if j >= heap_len {
                break;
            }

            // If right child exists and is smaller than left child, use right child
            if j + 1 < heap_len && self.smaller(tree, self.heap[j + 1] as usize, self.heap[j] as usize) {
                j += 1; // Move to right child
            }

            // If parent node v is smaller than smallest child, stop
            if self.smaller(tree, v as usize, self.heap[j] as usize) {
                break;
            }

            // Move the smallest child up
            self.heap[k] = self.heap[j];

            // Move down to child's position
            k = j;
        }

        self.heap[k] = v;
    }

    /// Compare two nodes in the heap based on frequencies and depths.
    /// Returns true if node `n` is "smaller" than node `m`.
    fn smaller(&self, tree: &[CtData], n: usize, m: usize) -> bool {
        tree[n].freq < tree[m].freq
            || (tree[n].freq == tree[m].freq && self.depth[n] <= self.depth[m])
    }

    fn build_bl_tree(&mut self, state: &GzipState) -> i32 {
        let mut max_blindex: i32;

        // Determine the bit length frequencies for literal and distance trees
        Self::scan_tree(&mut self.bl_tree, self.dyn_ltree.borrow_mut().deref_mut(), self.l_desc.max_code);
        Self::scan_tree(&mut self.bl_tree, self.dyn_dtree.borrow_mut().deref_mut(), self.d_desc.max_code);

        // Build the bit length tree
        self.build_tree(state, &mut self.bl_desc.clone());

        // At this point, opt_len includes the length of the tree representations,
        // except the lengths of the bit lengths codes and the 5+5+4 bits for the counts.

        // Determine the number of bit length codes to send.
        // The PKZIP format requires that at least 4 bit length codes be sent.
        max_blindex = (BL_CODES - 1) as i32;
        while max_blindex >= 3 {
            let code = BL_ORDER[max_blindex as usize];
            if self.bl_tree[code].len != 0 {
                break;
            }
            max_blindex -= 1;
        }

        // Update opt_len to include the bit length tree and counts
        self.opt_len = self.opt_len.wrapping_add(3 * ((max_blindex as u64) + 1) + 5 + 5 + 4);

        if state.verbose > 1 {
            eprintln!("\ndyn trees: dyn {}, stat {}", self.opt_len, self.static_len);
        }

        max_blindex
    }

    fn scan_tree(bl_tree: &mut [CtData; 2 * BL_CODES + 1], tree: &mut [CtData], max_code: i32) {
        let mut prevlen: i32 = -1;           // Last emitted length
        let mut curlen: i32;                 // Length of current code
        let mut nextlen: i32 = tree[0].len as i32; // Length of next code
        let mut count: i32 = 0;              // Repeat count of the current code
        let mut max_count: i32;              // Max repeat count
        let mut min_count: i32;              // Min repeat count

        if nextlen == 0 {
            max_count = 138;
            min_count = 3;
        } else {
            max_count = 7;
            min_count = 4;
        }

        // Set a guard value to prevent out-of-bounds access
        if (max_code + 1) as usize >= tree.len() {
            panic!("Tree array is too small");
        }
        tree[(max_code + 1) as usize].len = 0xFFFF;

        for n in 0..=max_code {
            let n = n as usize;
            curlen = nextlen;
            nextlen = tree[n + 1].len as i32;

            count += 1;

            if count < max_count && curlen == nextlen {
                continue;
            } else {
                if count < min_count {
                    // Update the frequency for the current code length
                    bl_tree[curlen as usize].freq += count as u16;
                } else if curlen != 0 {
                    if curlen != prevlen {
                        bl_tree[curlen as usize].freq += 1;
                    }
                    bl_tree[REP_3_6].freq += 1;
                } else if count <= 10 {
                    bl_tree[REPZ_3_10].freq += 1;
                } else {
                    bl_tree[REPZ_11_138].freq += 1;
                }

                count = 0;
                prevlen = curlen;

                if nextlen == 0 {
                    max_count = 138;
                    min_count = 3;
                } else if curlen == nextlen {
                    max_count = 6;
                    min_count = 3;
                } else {
                    max_count = 7;
                    min_count = 4;
                }
            }
        }
    }
}