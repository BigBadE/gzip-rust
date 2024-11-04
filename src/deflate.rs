use std::io;
use crate::GzipState;
use crate::trees::Trees;

const NIL: u16 = 0;
const HASH_SIZE: usize = 1 << 15; // Assuming HASH_BITS == 15
const WSIZE: usize = 32 * 1024; // Window size (32K)
const WMASK: usize = WSIZE - 1;
const MIN_LOOKAHEAD: usize = 262; // Minimum lookahead for deflate
pub(crate) const MIN_MATCH: usize = 3;
const FAST: u16 = 0x04;
const SLOW: u16 = 0x02;
pub(crate) const MAX_DIST: usize = 16384;
pub(crate) const MAX_MATCH: usize = 258;
const HASH_BITS: usize = 15;
const HASH_MASK: u32 = (HASH_SIZE as u32) - 1;
const WINDOW_SIZE: usize = 2*WSIZE;
const H_SHIFT: u32 = ((HASH_BITS + MIN_MATCH - 1) / MIN_MATCH) as u32; // 5
const CONFIGURATION_TABLE: [Config; 10] = [
    /* 0 */ Config::new(0, 0, 0, 0), /* store only */
    /* 1 */ Config::new(4, 4, 8, 4), /* maximum speed, no lazy matches */
    /* 2 */ Config::new(4, 5, 16, 8),
    /* 3 */ Config::new(4, 6, 32, 32),
    /* 4 */ Config::new(4, 4, 16, 16), /* lazy matches */
    /* 5 */ Config::new(8, 16, 32, 32),
    /* 6 */ Config::new(8, 16, 128, 128),
    /* 7 */ Config::new(8, 32, 128, 256),
    /* 8 */ Config::new(32, 128, 258, 1024),
    /* 9 */ Config::new(32, 258, 258, 4096)];

#[derive(Default)]
struct Config {
    max_lazy: i32,
    good_length: i32,
    nice_length: i32,
    max_chain: i32,
}

impl Config {
    const fn new(max_lazy: i32,
                 good_length: i32,
                 nice_length: i32,
                 max_chain: i32) -> Config {
        Self {
            max_lazy,
            good_length,
            nice_length,
            max_chain,
        }
    }
}
pub struct Deflate {
    compr_level: i32,
    head: [u16; HASH_SIZE],
    max_lazy_match: i32,
    good_match: i32,
    nice_match: i32,
    max_chain_length: i32,
    pub(crate) strstart: usize,
    pub(crate) block_start: i64,
    window: [u8; 2 * WSIZE],
    eofile: bool,
    lookahead: usize,
    ins_h: u32,
    prev: Vec<u16>,             // For maintaining previous positions
    prev_length: usize,
    match_start: usize,
    max_insert_length: usize,
}

impl Deflate {
    pub fn new() -> Self {
        Self {
            compr_level: 0,
            head: [NIL; HASH_SIZE],
            max_lazy_match: 0,
            good_match: 0,
            nice_match: 0,
            max_chain_length: 0,
            strstart: 0,
            block_start: 0,
            window: [0; 2 * WSIZE],
            eofile: false,
            lookahead: 0,
            ins_h: 0,
            prev: vec![0; WSIZE],
            prev_length: 0,
            match_start: 0,
            max_insert_length: 0
        }
    }

    pub fn lm_init(&mut self, state: &mut GzipState, pack_level: i32, flags: &mut u16) {
        if pack_level < 1 || pack_level > 9 {
            state.gzip_error("bad pack level");
        }
        self.compr_level = pack_level;

        // Initialize the hash table.
        self.head.fill(NIL);

        // prev will be initialized on the fly

        // Set the default configuration parameters:
        self.max_lazy_match = CONFIGURATION_TABLE[pack_level as usize].max_lazy;
        self.good_match = CONFIGURATION_TABLE[pack_level as usize].good_length;
        #[cfg(not(FULL_SEARCH))]
        {
            self.nice_match = CONFIGURATION_TABLE[pack_level as usize].nice_length;
        }
        self.max_chain_length = CONFIGURATION_TABLE[pack_level as usize].max_chain;

        if pack_level == 1 {
            *flags |= FAST;
        } else if pack_level == 9 {
            *flags |= SLOW;
        }

        self.strstart = 0;
        self.block_start = 0;

        (self.lookahead, self.eofile) = Self::read_buf(state, &mut self.window, 2 * WSIZE);

        if self.lookahead == 0 {
            self.eofile = true;
            self.lookahead = 0;
            return;
        }
        self.eofile = false;

        while self.lookahead < MIN_LOOKAHEAD && !self.eofile {
            self.fill_window(state);
        }

        self.ins_h = 0;
        for j in 0..(MIN_MATCH - 1) {
            self.ins_h = self.update_hash(self.ins_h, self.window[j]);
        }
    }

    fn update_hash(&self, h: u32, c: u8) -> u32 {
        // Implements UPDATE_HASH macro from C code
        (((h) << H_SHIFT) ^ (c as u32)) & HASH_MASK
    }

    fn read_buf(state: &mut GzipState, buf: &mut [u8], size: usize) -> (usize, bool) {
        if let Some(ref mut input) = state.ifd {
            match input.read(&mut buf[..size]) {
                Ok(bytes_read) => {
                    state.bytes_in += size as i64;
                    (bytes_read, bytes_read == 0)
                }
                Err(e) => {
                    state.gzip_error(&format!("Error reading input: {}", e));
                }
            }
        } else {
            (0, true)
        }
    }

    fn fill_window(&mut self, state: &mut GzipState) {
        // Move the existing data if necessary
        if self.strstart >= WSIZE + MAX_DIST {
            // Shift the window
            self.window.copy_within(WSIZE..2 * WSIZE, 0);
            self.strstart -= WSIZE;
            self.block_start -= WSIZE as i64;

            // Adjust the hash table
            for i in 0..HASH_SIZE {
                let h = self.head[i];
                if h as usize >= WSIZE {
                    self.head[i] = h - WSIZE as u16;
                } else {
                    self.head[i] = NIL;
                }
            }

            // Adjust the `prev` table
            for i in 0..WSIZE {
                let p = self.prev[i];
                if p as usize >= WSIZE {
                    self.prev[i] = p - WSIZE as u16;
                } else {
                    self.prev[i] = NIL;
                }
            }
        }

        // Read new data into the window
        let available_space = (2 * WSIZE) - self.lookahead - self.strstart;
        let (n, eof) = Self::read_buf(
            state,
            &mut self.window[self.strstart + self.lookahead..],
            available_space,
        );
        self.lookahead += n;
        self.eofile = eof;
        if n == 0 {
            self.eofile = true;
        }
    }

    pub fn deflate(&mut self, trees: &mut Trees, state: &mut GzipState) -> io::Result<()> {
        if self.compr_level <= 3 {
            return self.deflate_fast(trees, state);
        }

        unimplemented!()
    }

    pub fn deflate_fast(&mut self, tree: &mut Trees, state: &mut GzipState) -> io::Result<()> {
        let mut hash_head: usize = NIL as usize; // Head of the hash chain
        let mut flush: bool;            // Set if current block must be flushed
        let mut match_length: usize = 0; // Length of best match

        self.prev_length = MIN_MATCH - 1;
        while self.lookahead != 0 {
            // Insert the string window[strstart .. strstart+2] into the dictionary
            // and set hash_head to the head of the hash chain
            hash_head = self.insert_string(self.strstart);

            // Find the longest match, discarding those <= prev_length
            // At this point, we always have match_length < MIN_MATCH
            if hash_head != NIL.into()
                && self.strstart > hash_head
                && self.strstart - hash_head <= MAX_DIST
                && self.strstart <= WINDOW_SIZE - MIN_LOOKAHEAD
            {
                // To prevent matches with the string of window index 0
                match_length = self.longest_match(hash_head);
                // longest_match() sets self.match_start
                if match_length > self.lookahead {
                    match_length = self.lookahead;
                }
            }
            if match_length >= MIN_MATCH {
                self.check_match(state, self.strstart, self.match_start, match_length);

                flush = tree.ct_tally(self, state, self.strstart - self.match_start, match_length - MIN_MATCH);

                self.lookahead -= match_length;

                // Insert new strings in the hash table only if the match length is not too large
                if match_length <= self.max_insert_length {
                    match_length -= 1; // String at strstart already in hash table
                    while match_length != 0 {
                        self.strstart += 1;
                        hash_head = self.insert_string(self.strstart);
                        match_length -= 1;
                    }
                    self.strstart += 1;
                } else {
                    self.strstart += match_length;
                    match_length = 0;
                    self.ins_h = self.window[self.strstart] as u32;
                    self.ins_h = self.update_hash(self.ins_h, self.window[self.strstart + 1]);
                    // If MIN_MATCH != 3, call update_hash() MIN_MATCH - 3 more times
                    #[cfg(not(feature = "MIN_MATCH_3"))]
                    {
                        for i in 2..MIN_MATCH {
                            self.ins_h = self.update_hash(self.ins_h, self.window[self.strstart + i]);
                        }
                    }
                }
            } else {
                // No match, output a literal byte
                flush = tree.ct_tally(self, state, 0, self.window[self.strstart] as usize);
                self.lookahead -= 1;
                self.strstart += 1;
            }
            if flush {
                self.flush_block_wrapper(tree, state, false);
                self.block_start = self.strstart as i64;
            }

            // Ensure that we always have enough lookahead
            while self.lookahead < MIN_LOOKAHEAD && !self.eofile {
                self.fill_window(state);
            }
        }
        self.flush_block_wrapper(tree, state, true);
        Ok(())
    }

    fn flush_block_wrapper(&mut self, trees: &mut Trees, state: &mut GzipState, eof: bool) -> i64 {
        if self.block_start >= 0 {
            let start = self.block_start as usize;
            let end = self.strstart;

            // Ensure indices are within the bounds of the window
            if start <= end && end <= self.window.len() {
                let buf = &self.window[start..end];
                let stored_len = end - start;
                trees.flush_block(state, Some(buf), stored_len as u64, eof)
            } else {
                // Handle invalid indices
                panic!("flush_block_wrapper: Invalid window indices");
            }
        } else {
            // block_start < 0
            let stored_len = 0;
            trees.flush_block(state, None, stored_len, eof)
        }
    }

    fn insert_string(&mut self, s: usize) -> usize {
        // Corresponds to the INSERT_STRING macro
        self.ins_h = self.update_hash(self.ins_h, self.window[s + MIN_MATCH - 1]);
        let ins_h = self.ins_h as usize;
        let match_head = self.head[ins_h] as usize;
        self.prev[s & WMASK] = match_head as u16;
        self.head[ins_h] = s as u16;
        match_head
    }

    fn longest_match(&mut self, mut cur_match: usize) -> usize {
        let mut chain_length = self.max_chain_length; // Max hash chain length
        let scan = self.strstart;                     // Current string position
        let mut best_len = self.prev_length;          // Best match length so far
        let limit = if self.strstart > MAX_DIST {
            self.strstart - MAX_DIST
        } else {
            0
        };
        // Stop when cur_match becomes <= limit. To simplify the code,
        // we prevent matches with the string of window index 0.

        // Do not waste too much time if we already have a good match:
        if self.prev_length >= self.good_match as usize {
            chain_length >>= 2;
        }
        assert!(
            self.strstart <= WINDOW_SIZE - MIN_LOOKAHEAD,
            "insufficient lookahead"
        );

        let window = &self.window;
        let window_size = WINDOW_SIZE;
        let mut nice_match = self.nice_match as usize;

        let strend = self.strstart + MAX_MATCH;
        let mut scan_end1 = window[scan + best_len - 1];
        let mut scan_end = window[scan + best_len];

        while cur_match > limit && chain_length != 0 {
            chain_length -= 1;
            let match_index = cur_match;

            // Skip to next match if the match length cannot increase
            // or if the match length is less than 2:
            if window[match_index + best_len] != scan_end
                || window[match_index + best_len - 1] != scan_end1
                || window[match_index] != window[scan]
                || window[match_index + 1] != window[scan + 1]
            {
                cur_match = self.prev[cur_match & WMASK] as usize;
                continue;
            }

            // Now, try to match as much as possible
            let mut len = 2;
            while len < MAX_MATCH
                && window[scan + len] == window[match_index + len]
            {
                len += 1;
            }

            if len > best_len {
                self.match_start = match_index;
                best_len = len;
                if len >= nice_match {
                    break;
                }
                if len >= scan + best_len {
                    scan_end1 = window[scan + best_len - 1];
                    scan_end = window[scan + best_len];
                }
            }

            cur_match = self.prev[cur_match & WMASK] as usize;
        }

        best_len
    }

    fn check_match(&self, state: &GzipState, start: usize, match_pos: usize, length: usize) {
        // Check that the match is indeed a match
        let window = &self.window;

        // Ensure indices are within bounds
        if start + length > window.len() || match_pos + length > window.len() {
            eprintln!("Index out of bounds in check_match");
            state.gzip_error("invalid match");
        }

        if &window[match_pos..match_pos + length] != &window[start..start + length] {
            eprintln!(" start {}, match {}, length {}", start, match_pos, length);
            state.gzip_error("invalid match");
        }

        if state.verbose > 1 {
            eprint!("\\[{},{}]", start - match_pos, length);
            for &byte in &window[start..start + length] {
                eprint!("{}", byte as char);
            }
        }
    }
}