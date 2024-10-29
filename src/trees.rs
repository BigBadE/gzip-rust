use crate::deflate::{Deflate, MAX_DIST, MAX_MATCH, MIN_MATCH};
use crate::{GzipState, STORED};

const MAX_BITS: usize = 15;
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

#[derive(Default, Copy, Clone)]
struct CtData {
    freq: u16,
    len: u16,
    code: u16,
}

pub struct Trees<'a> {
    file_type: Option<&'a mut u16>,
    file_method: Option<&'a mut i32>,
    compressed_len: i64,
    input_len: i64,
    base_length: [i32; LENGTH_CODES],
    base_dist: [i32; D_CODES],
    length_code: [u8; 256],
    dist_code: [u8; 512],
    bl_count: [i32; MAX_BITS + 1],
    static_ltree: [CtData; L_CODES + 2],
    static_dtree: [CtData; D_CODES],
    dyn_ltree: [CtData; HEAP_SIZE],
    dyn_dtree: [CtData; 2 * D_CODES + 1],
    bl_tree: [CtData; 2 * BL_CODES + 1],
    opt_len: i64,
    static_len: i64,
    last_lit: i32,
    last_dist: i32,
    last_flags: i32,
    flags: i32,
    flag_bit: i32,
    l_buf: [usize; LIT_BUFSIZE],
    d_buf: [usize; DIST_BUFSIZE],
    flag_buf: [usize; LIT_BUFSIZE/8],
}

impl<'a> Trees<'a> {
    pub fn new() -> Self {
        Self {
            file_type: None,
            file_method: None,
            compressed_len: 0,
            input_len: 0,
            base_length: [0; LENGTH_CODES],
            base_dist: [0; D_CODES],
            length_code: [0; 256],
            dist_code: [0; 512],
            bl_count: [0; MAX_BITS + 1],
            static_ltree: [CtData::default(); L_CODES + 2],
            static_dtree: [CtData::default(); D_CODES],
            dyn_ltree: [CtData::default(); HEAP_SIZE],
            dyn_dtree: [CtData::default(); 2 * D_CODES + 1],
            bl_tree: [CtData::default(); 2 * BL_CODES + 1],
            opt_len: 0,
            static_len: 0,
            last_lit: 0,
            last_dist: 0,
            last_flags: 0,
            flags: 0,
            flag_bit: 1,
            l_buf: [0; LIT_BUFSIZE],
            d_buf: [0; DIST_BUFSIZE],
            flag_buf: [0; LIT_BUFSIZE/8],
        }
    }

    pub(crate) fn ct_init(&mut self, attr: &'a mut u16, methodp: &'a mut i32) {
        let mut n: i32;
        let mut length: i32;
        let mut code: i32;
        let mut dist: i32;

        self.file_type = Some(attr);
        self.file_method = Some(methodp);
        self.compressed_len = 0;
        self.input_len = 0;

        if self.static_dtree[0].len != 0 {
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
            self.static_ltree[n as usize].len = 8;
            self.bl_count[8] += 1;
            n += 1;
        }
        while n <= 255 {
            self.static_ltree[n as usize].len = 9;
            self.bl_count[9] += 1;
            n += 1;
        }
        while n <= 279 {
            self.static_ltree[n as usize].len = 7;
            self.bl_count[7] += 1;
            n += 1;
        }
        while n <= 287 {
            self.static_ltree[n as usize].len = 8;
            self.bl_count[8] += 1;
            n += 1;
        }

        // Generate the codes
        Self::gen_codes(&self.bl_count, &mut self.static_ltree, (L_CODES + 1) as i32);

        // The static distance tree is trivial
        for n in 0..D_CODES as i32 {
            self.static_dtree[n as usize].len = 5;
            self.static_dtree[n as usize].code = Self::bi_reverse(n as u16, 5);
        }

        // Initialize the first block of the first file
        self.init_block();
    }

    fn init_block(&mut self) {
        // Initialize the dynamic literal tree frequencies
        for n in 0..L_CODES {
            self.dyn_ltree[n].freq = 0;
        }

        // Initialize the dynamic distance tree frequencies
        for n in 0..D_CODES {
            self.dyn_dtree[n].freq = 0;
        }

        // Initialize the bit length tree frequencies
        for n in 0..BL_CODES {
            self.bl_tree[n].freq = 0;
        }

        // Set the frequency of the END_BLOCK symbol to 1
        self.dyn_ltree[END_BLOCK].freq = 1;
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
            self.dyn_ltree[lc].freq += 1;
        } else {
            // lc is the match length - MIN_MATCH
            let dist = dist - 1; // Adjust distance
            assert!(
                dist < MAX_DIST
                    && lc <= MAX_MATCH - MIN_MATCH
                    && self.d_code(dist) < D_CODES,
                "ct_tally: bad match"
            );

            self.dyn_ltree[self.length_code[lc] as usize + LITERALS + 1].freq += 1;
            self.dyn_dtree[self.d_code(dist)].freq += 1;

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
                out_length += self.dyn_dtree[dcode].freq as u64
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

    fn flush_block(
        &mut self,
        state: &mut GzipState,
        buf: Option<&[u8]>,
        stored_len: u64,
        eof: bool,
    ) -> i64 {
        let mut opt_lenb: u64;
        let mut static_lenb: u64;
        let mut max_blindex: i32;

        // Save the flags for the last 8 items
        self.flag_buf[self.last_flags as usize] = self.flags as usize;

        // Check if the file is ASCII or binary
        if self.file_type == None {
            self.set_file_type();
        }

        // Construct the literal and distance trees
        self.build_tree(&mut self.l_desc);
        if state.verbose > 1 {
            eprintln!(
                "\nlit data: dyn {}, stat {}",
                self.opt_len, self.static_len
            );
        }

        self.build_tree(&mut self.d_desc);
        if state.verbose > 1 {
            eprintln!(
                "\ndist data: dyn {}, stat {}",
                self.opt_len, self.static_len
            );
        }

        // Build the bit length tree and get the index of the last bit length code to send
        max_blindex = self.build_bl_tree();

        // Determine the best encoding. Compute the block length in bytes
        opt_lenb = (self.opt_len + 3 + 7) >> 3;
        static_lenb = (self.static_len + 3 + 7) >> 3;
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

        if stored_len <= opt_lenb && eof && self.compressed_len == 0 && self.seekable() {
            // Since LIT_BUFSIZE <= 2*WSIZE, the input data must be there
            if buf.is_none() {
                self.gzip_error("block vanished");
            }

            self.copy_block(buf.unwrap(), stored_len as usize, false); // Without header
            self.compressed_len = stored_len << 3;
            self.file_method = STORED;
        } else if stored_len + 4 <= opt_lenb && buf.is_some() {
            // 4: two words for the lengths
            let eof_flag = if eof { 1 } else { 0 };
            self.send_bits(((STORED_BLOCK << 1) + eof_flag) as u16, 3); // Send block type
            self.compressed_len = (self.compressed_len + 3 + 7) & !7u64;
            self.compressed_len += ((stored_len + 4) << 3) as i64;

            self.copy_block(buf.unwrap(), stored_len as usize, true); // With header
        } else if static_lenb == opt_lenb {
            let eof_flag = if eof { 1 } else { 0 };
            self.send_bits(((STATIC_TREES << 1) + eof_flag) as u16, 3);
            self.compress_block(&self.static_ltree, &self.static_dtree);
            self.compressed_len += 3 + self.static_len;
        } else {
            let eof_flag = if eof { 1 } else { 0 };
            self.send_bits(((DYN_TREES << 1) + eof_flag) as u16, 3);
            self.send_all_trees(
                self.l_desc.max_code + 1,
                self.d_desc.max_code + 1,
                max_blindex + 1,
            );
            self.compress_block(&self.dyn_ltree, &self.dyn_dtree);
            self.compressed_len += 3 + self.opt_len;
        }

        assert!(
            self.compressed_len == self.bits_sent,
            "bad compressed size"
        );
        self.init_block();

        if eof {
            assert!(self.input_len == state.bytes_in, "bad input size");
            self.bi_windup();
            self.compressed_len += 7; // Align on byte boundary
        }

        (self.compressed_len >> 3) as i64
    }
}