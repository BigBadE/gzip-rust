mod trees;
mod zip;
mod deflate;
mod inflate;
mod unzip;

use crate::zip::zip;
use crate::unzip::unzip;
use byteorder::{LittleEndian, ReadBytesExt};
use chrono::{DateTime, Datelike, Local, Timelike};
// use crc::{Crc, Digest, CRC_16_IBM_SDLC};
use crc::{Crc, CRC_32_ISO_HDLC};
use std::collections::HashSet;
use std::fs::{File, Metadata};
use std::io::{stdout, Read, Write};
use std::path::{Path, PathBuf};
use std::process::exit;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime};
use std::{env, fs, io};
use ::deflate::{checksum, deflate_bytes, Compression, CompressionOptions};
use ::deflate::compress::Flush;
use ::deflate::deflate_state::DeflateState;
use ::deflate::writer::compress_until_done;

// Constants (Assumed values for any not defined in the provided C code)
const BITS: i32 = 16; // Assuming 16 bits
const DEFLATED: i32 = 8;
const OK: i32 = 0;
const ERROR: i32 = 1;
const MAX_PATH_LEN: usize = 1024; // As defined in the C code
const Z_SUFFIX: &str = ".gz";
const MAX_SUFFIX: usize = 30; // Assuming maximum suffix length

const VERSION: &str = "1.0"; // Assuming version 1.0, replace with actual version.

#[cfg(all(target_os = "windows", target_pointer_width = "32"))]
const OS_CODE: u8 = 0x0b;
#[cfg(target_os = "macos")]
const OS_CODE: u8 = 0x07;
#[cfg(all(not(all(target_os = "windows", target_pointer_width = "32")), not(target_os = "macos")))]
const OS_CODE: u8 = 0x03;

const CRC_32_TAB: [u32; 256] = [
  0x00000000, 0x77073096, 0xee0e612c, 0x990951ba, 0x076dc419,
  0x706af48f, 0xe963a535, 0x9e6495a3, 0x0edb8832, 0x79dcb8a4,
  0xe0d5e91e, 0x97d2d988, 0x09b64c2b, 0x7eb17cbd, 0xe7b82d07,
  0x90bf1d91, 0x1db71064, 0x6ab020f2, 0xf3b97148, 0x84be41de,
  0x1adad47d, 0x6ddde4eb, 0xf4d4b551, 0x83d385c7, 0x136c9856,
  0x646ba8c0, 0xfd62f97a, 0x8a65c9ec, 0x14015c4f, 0x63066cd9,
  0xfa0f3d63, 0x8d080df5, 0x3b6e20c8, 0x4c69105e, 0xd56041e4,
  0xa2677172, 0x3c03e4d1, 0x4b04d447, 0xd20d85fd, 0xa50ab56b,
  0x35b5a8fa, 0x42b2986c, 0xdbbbc9d6, 0xacbcf940, 0x32d86ce3,
  0x45df5c75, 0xdcd60dcf, 0xabd13d59, 0x26d930ac, 0x51de003a,
  0xc8d75180, 0xbfd06116, 0x21b4f4b5, 0x56b3c423, 0xcfba9599,
  0xb8bda50f, 0x2802b89e, 0x5f058808, 0xc60cd9b2, 0xb10be924,
  0x2f6f7c87, 0x58684c11, 0xc1611dab, 0xb6662d3d, 0x76dc4190,
  0x01db7106, 0x98d220bc, 0xefd5102a, 0x71b18589, 0x06b6b51f,
  0x9fbfe4a5, 0xe8b8d433, 0x7807c9a2, 0x0f00f934, 0x9609a88e,
  0xe10e9818, 0x7f6a0dbb, 0x086d3d2d, 0x91646c97, 0xe6635c01,
  0x6b6b51f4, 0x1c6c6162, 0x856530d8, 0xf262004e, 0x6c0695ed,
  0x1b01a57b, 0x8208f4c1, 0xf50fc457, 0x65b0d9c6, 0x12b7e950,
  0x8bbeb8ea, 0xfcb9887c, 0x62dd1ddf, 0x15da2d49, 0x8cd37cf3,
  0xfbd44c65, 0x4db26158, 0x3ab551ce, 0xa3bc0074, 0xd4bb30e2,
  0x4adfa541, 0x3dd895d7, 0xa4d1c46d, 0xd3d6f4fb, 0x4369e96a,
  0x346ed9fc, 0xad678846, 0xda60b8d0, 0x44042d73, 0x33031de5,
  0xaa0a4c5f, 0xdd0d7cc9, 0x5005713c, 0x270241aa, 0xbe0b1010,
  0xc90c2086, 0x5768b525, 0x206f85b3, 0xb966d409, 0xce61e49f,
  0x5edef90e, 0x29d9c998, 0xb0d09822, 0xc7d7a8b4, 0x59b33d17,
  0x2eb40d81, 0xb7bd5c3b, 0xc0ba6cad, 0xedb88320, 0x9abfb3b6,
  0x03b6e20c, 0x74b1d29a, 0xead54739, 0x9dd277af, 0x04db2615,
  0x73dc1683, 0xe3630b12, 0x94643b84, 0x0d6d6a3e, 0x7a6a5aa8,
  0xe40ecf0b, 0x9309ff9d, 0x0a00ae27, 0x7d079eb1, 0xf00f9344,
  0x8708a3d2, 0x1e01f268, 0x6906c2fe, 0xf762575d, 0x806567cb,
  0x196c3671, 0x6e6b06e7, 0xfed41b76, 0x89d32be0, 0x10da7a5a,
  0x67dd4acc, 0xf9b9df6f, 0x8ebeeff9, 0x17b7be43, 0x60b08ed5,
  0xd6d6a3e8, 0xa1d1937e, 0x38d8c2c4, 0x4fdff252, 0xd1bb67f1,
  0xa6bc5767, 0x3fb506dd, 0x48b2364b, 0xd80d2bda, 0xaf0a1b4c,
  0x36034af6, 0x41047a60, 0xdf60efc3, 0xa867df55, 0x316e8eef,
  0x4669be79, 0xcb61b38c, 0xbc66831a, 0x256fd2a0, 0x5268e236,
  0xcc0c7795, 0xbb0b4703, 0x220216b9, 0x5505262f, 0xc5ba3bbe,
  0xb2bd0b28, 0x2bb45a92, 0x5cb36a04, 0xc2d7ffa7, 0xb5d0cf31,
  0x2cd99e8b, 0x5bdeae1d, 0x9b64c2b0, 0xec63f226, 0x756aa39c,
  0x026d930a, 0x9c0906a9, 0xeb0e363f, 0x72076785, 0x05005713,
  0x95bf4a82, 0xe2b87a14, 0x7bb12bae, 0x0cb61b38, 0x92d28e9b,
  0xe5d5be0d, 0x7cdcefb7, 0x0bdbdf21, 0x86d3d2d4, 0xf1d4e242,
  0x68ddb3f8, 0x1fda836e, 0x81be16cd, 0xf6b9265b, 0x6fb077e1,
  0x18b74777, 0x88085ae6, 0xff0f6a70, 0x66063bca, 0x11010b5c,
  0x8f659eff, 0xf862ae69, 0x616bffd3, 0x166ccf45, 0xa00ae278,
  0xd70dd2ee, 0x4e048354, 0x3903b3c2, 0xa7672661, 0xd06016f7,
  0x4969474d, 0x3e6e77db, 0xaed16a4a, 0xd9d65adc, 0x40df0b66,
  0x37d83bf0, 0xa9bcae53, 0xdebb9ec5, 0x47b2cf7f, 0x30b5ffe9,
  0xbdbdf21c, 0xcabac28a, 0x53b39330, 0x24b4a3a6, 0xbad03605,
  0xcdd70693, 0x54de5729, 0x23d967bf, 0xb3667a2e, 0xc4614ab8,
  0x5d681b02, 0x2a6f2b94, 0xb40bbe37, 0xc30c8ea1, 0x5a05df1b,
  0x2d02ef8d];


const LICENSE_MSG: &[&str] = &[
    "Copyright (C) 2007, 2010, 2011 Free Software Foundation, Inc.",
    "Copyright (C) 1993 Jean-loup Gailly.",
    "This is free software.  You may redistribute copies of it under the terms of",
    "the GNU General Public License <http://www.gnu.org/licenses/gpl.html>.",
    "There is NO WARRANTY, to the extent permitted by law.",
];

// const CRC16: Crc<u16> = Crc::<u16>::new(&CRC_16_IBM_SDLC);
const CRC32: Crc<u32> = Crc::<u32>::new(&CRC_32_ISO_HDLC);
//
// Magic headers
const PACK_MAGIC: &[u8] = b"\x1F\x1E"; // Magic header for packed files
const GZIP_MAGIC: &[u8] = b"\x1F\x8B"; // Magic header for gzip files, 1F 8B
const OLD_GZIP_MAGIC: &[u8] = b"\x1F\x9E"; // Magic header for gzip 0.5 = freeze 1.x
const LZH_MAGIC: &[u8] = b"\x1F\xA0"; // Magic header for SCO LZH Compress files
const LZW_MAGIC: &[u8] = b"\x1F\x9D"; // Magic header for SCO LZW Compress files
const PKZIP_MAGIC: &[u8] = b"\x50\x4B\x03\x04"; // Magic header for pkzip files

// gzip flag bytes
const HEADER_CRC: u8 = 0x02; // bit 1 set: CRC16 for the gzip header
const EXTRA_FIELD: u8 = 0x04; // bit 2 set: extra field present
const ORIG_NAME: u8 = 0x08; // bit 3 set: original file name present
const COMMENT: u8 = 0x10; // bit 4 set: file comment present
const ENCRYPTED: u8 = 0x20; // bit 5 set: file is encrypted
const RESERVED: u8 = 0xC0; // bits 6 and 7: reserved
const WSIZE: usize = 0x8000;
const INBUFSIZ: usize = 0x8000;
const OUTBUFSIZ: usize = 16384;
const INBUF_EXTRA: usize = 64;
const OUTBUF_EXTRA: usize = 2048;
const DIST_BUFSIZE: usize = 0x8000;
const STORED: i32 = 0;
const COMPRESSED: u8 = 1;
const PACKED: u8 = 2;
const LZHED: u8 = 3;
const MAX_METHODS: usize = 9;
const HELP_MSG: &[&str] = &[
    "Compress or uncompress FILEs (by default, compress FILES in-place).",
    "",
    "Mandatory arguments to long options are mandatory for short options too.",
    "",
    // Assuming O_BINARY is false (platform-independent code)
    "  -a, --ascii       ascii text; convert end-of-line using local conventions",
    "  -c, --stdout      write on standard output, keep original files unchanged",
    "  -d, --decompress  decompress",
    "  -f, --force       force overwrite of output file and compress links",
    "  -h, --help        give this help",
    "  -k, --keep        keep (don't delete) input files",
    "  -l, --list        list compressed file contents",
    "  -L, --license     display software license",
    "  -n, --no-name     do not save or restore the original name and timestamp",
    "  -N, --name        save or restore the original name and timestamp",
    "  -q, --quiet       suppress all warnings",
    // Assuming directories are supported
    "  -r, --recursive   operate recursively on directories",
    "      --rsyncable   make rsync-friendly archive",
    "  -S, --suffix=SUF  use suffix SUF on compressed files",
    "      --synchronous synchronous output (safer if system crashes, but slower)",
    "  -t, --test        test compressed file integrity",
    "  -v, --verbose     verbose mode",
    "  -V, --version     display version number",
    "  -1, --fast        compress faster",
    "  -9, --best        compress better",
    "",
    "With no FILE, or when FILE is -, read standard input.",
    "",
    "Report bugs to <bug-gzip@gnu.org>.",
];
static COUNT: AtomicUsize = AtomicUsize::new(0);

// The main state structure encapsulating all the global variables
struct GzipState {
    // Options and flags
    presume_input_tty: bool,
    ascii: bool,
    to_stdout: bool,
    decompress: bool,
    force: i32,
    keep: bool,
    no_name: Option<bool>, // None represents -1 in C code
    no_time: Option<bool>,
    recursive: bool,
    list: bool,
    verbose: i32,
    quiet: bool,
    do_lzw: bool,
    test: bool,
    _foreground: bool,
    // Program state
    program_name: String,
    _env: Option<String>,
    args: Vec<String>,
    z_suffix: String,
    z_len: usize,
    exit_code: i32,
    maxbits: i32,
    method: i32,
    level: i32,
    save_orig_name: bool,
    last_member: bool,
    part_nb: i32,
    time_stamp: Option<SystemTime>,
    ifile_size: i64,
    _caught_signals: HashSet<i32>,
    _exiting_signal: Option<i32>,
    _remove_ofname_fd: Option<i32>,
    pub bytes_in: i64,
    bytes_out: i64,
    total_in: i64,
    total_out: i64,
    ifname: String,
    ofname: String,
    istat: Option<Metadata>,
    ifd: Option<Box<dyn Read>>,
    ofd: Option<Box<dyn Write>>,
    insize: usize,
    inptr: usize,
    outcnt: usize,
    _handled_sig: Vec<i32>,
    header_bytes: usize,
    // Function pointer for the current operation
    work: Option<fn(&mut GzipState) -> io::Result<()>>,
    inbuf: [u8; INBUFSIZ + INBUF_EXTRA], // Input buffer
    outbuf: [u8; OUTBUFSIZ + OUTBUF_EXTRA], // Output buffer
    window: [u8; 2 * WSIZE], // Output buffer
    crc16_digest: u32,
    first_time: bool,
    record_io: bool,
    bi_buf: u16,
    bi_valid: u8,
    first_call: bool,
}

// Implementation of the GzipState struct
impl GzipState {


    fn new() -> Self {
        let program_name = env::args().next().unwrap_or_else(|| "gzip".to_string())
            .split(".").next().unwrap()
            .split("/").last().unwrap()
            .split("\\").last().unwrap().to_string();
        GzipState {
            presume_input_tty: false,
            ascii: false,
            to_stdout: false,
            decompress: false,
            force: 0,
            keep: false,
            no_name: None, // None represents -1 (undefined) in the C code
            no_time: None, // None represents -1 (undefined) in the C code
            recursive: false,
            list: false,
            verbose: 0,
            quiet: false,
            do_lzw: false,
            test: false,
            _foreground: false,
            program_name,
            _env: None,
            args: vec![],
            z_suffix: Z_SUFFIX.to_string(),
            z_len: Z_SUFFIX.len(),
            exit_code: OK,
            maxbits: BITS,
            method: DEFLATED,
            level: 6,
            save_orig_name: false,
            last_member: false,
            part_nb: 0,
            time_stamp: None,
            ifile_size: -1,
            _caught_signals: HashSet::new(),
            _exiting_signal: None,
            _remove_ofname_fd: None,
            bytes_in: 0,
            bytes_out: 0,
            total_in: 0,
            total_out: 0,
            ifname: String::new(),
            ofname: String::new(),
            istat: None,
            ifd: None,
            ofd: None,
            insize: 0,
            inptr: 0,
            outcnt: 0,
            _handled_sig: vec![],
            header_bytes: 0,
            work: None, // Function pointer will be set during runtime
            inbuf: [0; INBUFSIZ + INBUF_EXTRA],
            outbuf: [0; OUTBUFSIZ + OUTBUF_EXTRA],
            window: [0; 2 * WSIZE],
            crc16_digest: 0x00000000,
            first_time: false,
            record_io: false,
            bi_buf: 0,
            bi_valid: 0,
            first_call: true,
        }
    }

    // Example method to set the 'work' function pointer based on the operation
    fn set_work_function(&mut self) {
        if self.decompress {
            self.work = Some(unzip); // Assuming 'unzip' is defined elsewhere
        } else if self.do_lzw {
            self.work = Some(lzw); // Assuming 'lzw' is defined elsewhere
        } else {
            self.work = Some(zip); // Assuming 'zip' is defined elsewhere
        }
    }

    // Other methods to manipulate the state can be added here
    // Function to perform cleanup and exit
    fn do_exit(&self, exitcode: i32) -> ! {
        // Perform any necessary cleanup here.
        // In Rust, resources are automatically cleaned up when they go out of scope,
        // so explicit cleanup may not be necessary unless using unsafe code or raw pointers.

        exit(exitcode);
    }

    // Translated try_help function
    fn try_help(&self) -> ! {
        eprintln!("Try `{} --help' for more information.", self.program_name);
        self.do_exit(ERROR);
    }

    fn help(&self) {
        println!("Usage: {} [OPTION]... [FILE]...", self.program_name);
        for line in HELP_MSG {
            println!("{}", line);
        }
    }

    fn license(&self) {
        println!("{} {}", self.program_name, VERSION);
        for line in LICENSE_MSG {
            println!("{}", line);
        }
    }

    fn version(&self) {
        self.license();
        println!();
        println!("Written by Jean-loup Gailly.");
    }

    fn progerror(&mut self, path: &Path) {
        eprintln!("{}: {}", self.program_name, path.display());
        self.exit_code = ERROR;
    }

    // Function to parse command-line arguments
    fn parse_args(&mut self) {
        let args: Vec<String> = env::args().collect();
        let mut arg_iter = args.iter().skip(1).peekable();

        while let Some(arg) = arg_iter.next() {
            if arg.starts_with('-') && arg.len() > 1 {
                match &arg[1..] {
                    "a" => self.ascii = true,
                    "b" => {
                        if let Some(bits_arg) = arg_iter.next() {
                            self.maxbits = bits_arg.parse().unwrap_or_else(|_| {
                                eprintln!("{}: -b operand is not an integer", self.program_name);
                                self.try_help();
                            });
                        } else {
                            eprintln!("{}: -b requires an operand", self.program_name);
                            self.try_help();
                        }
                    }
                    "c" => self.to_stdout = true,
                    "d" => self.decompress = true,
                    "f" => self.force += 1,
                    "h" | "H" => {
                        self.help();
                        self.do_exit(OK);
                    }
                    "k" => self.keep = true,
                    "l" => {
                        self.list = true;
                        self.decompress = true;
                        self.to_stdout = true;
                    }
                    "L" => {
                        self.license();
                        self.do_exit(OK);
                    }
                    "m" => self.no_time = Some(true),
                    "M" => self.no_time = Some(false),
                    "n" => {
                        self.no_name = Some(true);
                        self.no_time = Some(true);
                    }
                    "N" => {
                        self.no_name = Some(false);
                        self.no_time = Some(false);
                    }
                    "q" => {
                        self.quiet = true;
                        self.verbose = 0;
                    }
                    "r" => self.recursive = true,
                    "S" => {
                        if let Some(suffix_arg) = arg_iter.next() {
                            self.z_suffix = suffix_arg.clone();
                            self.z_len = self.z_suffix.len();
                        } else {
                            eprintln!("{}: -S requires a suffix", self.program_name);
                            self.try_help();
                        }
                    }
                    "t" => {
                        self.test = true;
                        self.decompress = true;
                        self.to_stdout = true;
                    }
                    "v" => {
                        self.verbose += 1;
                        self.quiet = false;
                    }
                    "V" => {
                        self.version();
                        self.do_exit(OK);
                    }
                    "Z" => self.do_lzw = true,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" => {
                        self.level = arg[1..].parse::<i32>().unwrap();
                    }
                    _ => {
                        eprintln!("{}: unknown option -- '{}'", self.program_name, &arg[1..]);
                        self.try_help();
                    }
                }
            } else {
                self.args.push(arg.clone());
            }
        }
    }

    // Implement other methods like help, try_help, do_exit, license, version...
    // For brevity, let's assume they are already implemented as in previous translations

    // Entry point to start processing files or stdin
    fn run(&mut self) -> io::Result<()> {
        // By default, save name and timestamp on compression but do not restore them on decompression.
        if self.no_time.is_none() {
            self.no_time = Some(self.decompress);
        }
        if self.no_name.is_none() {
            self.no_name = Some(self.decompress);
        }

        if self.z_len == 0 || self.z_len > MAX_SUFFIX {
            eprintln!("{}: invalid suffix '{}'", self.program_name, self.z_suffix);
            self.do_exit(ERROR);
        }

        // Set work function based on options
        self.set_work_function();

        // Install signal handlers (if necessary)
        self.install_signal_handlers();

        // Process files
        if !self.args.is_empty() {
            if self.to_stdout && !self.test && !self.list && (!self.decompress || !self.ascii) {
                // Set stdout to binary mode if necessary
                // In Rust, stdout is typically in binary mode
            }
            for filename in self.args.clone() {
                self.treat_file(&filename)?;
            }
        } else {
            // Process standard input
            self.treat_stdin()?;
        }

        if self.list && !self.quiet && self.args.len() > 1 {
            self.do_list::<File>(None, -1)?; // Print totals
        }

        self.do_exit(self.exit_code);
    }

    // Placeholder for treat_file function
    fn treat_file(&mut self, iname: &str) -> io::Result<()> {
        if iname == "-" {
            let cflag = self.to_stdout;
            self.treat_stdin()?; // Assume treat_stdin is implemented
            self.to_stdout = cflag;
            return Ok(());
        }

        let path = Path::new(iname);
        self.ifname = iname.to_string();

        let metadata = match fs::metadata(path) {
            Ok(meta) => meta,
            Err(err) => {
                eprintln!("{}: {}", self.program_name, err);
                return Ok(());
            }
        };
        self.istat = Some(metadata.clone());

        if metadata.is_dir() {
            if self.recursive {
                self.treat_dir(path)?; // Assume treat_dir is implemented
                // Warning: ifname is now invalid
                return Ok(());
            } else {
                eprintln!("{}: {} is a directory -- ignored", self.program_name, self.ifname);
                return Ok(());
            }
        }

        if !self.to_stdout {
            if !metadata.is_file() {
                eprintln!(
                    "{}: {} is not a directory or a regular file -- ignored",
                    self.program_name, self.ifname
                );
                return Ok(());
            }

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = metadata.permissions().mode();

                if (mode & 0o4000) != 0 {
                    eprintln!(
                        "{}: {} is set-user-ID on execution -- ignored",
                        self.program_name, self.ifname
                    );
                    return Ok(());
                }
                if (mode & 0o2000) != 0 {
                    eprintln!(
                        "{}: {} is set-group-ID on execution -- ignored",
                        self.program_name, self.ifname
                    );
                    return Ok(());
                }

                if self.force == 0 {
                    if (mode & 0o1000) != 0 {
                        eprintln!(
                            "{}: {} has the sticky bit set -- file ignored",
                            self.program_name, self.ifname
                        );
                        return Ok(());
                    }
                    if metadata.nlink() >= 2 {
                        let other_links = metadata.nlink() - 1;
                        eprintln!(
                            "{}: {} has {} other link{} -- unchanged",
                            self.program_name,
                            self.ifname,
                            other_links,
                            if other_links == 1 { "" } else { "s" }
                        );
                        return Ok(());
                    }
                }
            }
        }

        self.ifile_size = if metadata.is_file() {
            metadata.len() as i64
        } else {
            -1
        };

        if !self.no_time.unwrap_or(false) || self.list {
            self.time_stamp = metadata.modified().ok();
        }

        if self.to_stdout && !self.list && !self.test {
            self.ofname = "stdout".to_string();
        } else if self.make_ofname().is_err() {
            return Ok(());
        }

        self.clear_bufs();
        self.part_nb = 0;

        let mut ifd = match File::open(path) {
            Ok(file) => file,
            Err(err) => {
                eprintln!("{}: {}", self.program_name, err);
                return Ok(());
            }
        };

        if self.decompress {
            self.method = match self.get_method(&mut ifd)? {
                Some(method) => method,
                None => {
                    return Ok(());
                }
            };
        }

        if self.list {
            self.do_list(Some(&mut ifd), self.method)?; // Assume do_list is implemented
            return Ok(());
        }

        if self.to_stdout {
            self.ofd = Some(Box::new(io::stdout()));
        } else {
            self.ofd = Some(Box::new(self.create_outfile()?));
            if !self.decompress && self.save_orig_name && self.verbose == 0 && !self.quiet {
                println!(
                    "{}: {} compressed to {}",
                    self.program_name, self.ifname, self.ofname
                );
            }
        }

        if !self.save_orig_name {
            self.save_orig_name = !self.no_name.unwrap_or(false);
        }

        if self.verbose != 0 {
            eprint!("{}:\t", self.ifname);
        }

        loop {
            if let Some(work_fn) = self.work {
                self.ifd = Some(Box::new(ifd.try_clone()?));
                if work_fn(self).is_err() {
                    self.method = -1;
                    break;
                }
            } else {
                eprintln!("{}: work function not set", self.program_name);
                return Ok(());
            }

            if self.input_eof()? {
                break;
            }

            self.method = match self.get_method(&mut ifd)? {
                Some(method) => method,
                None => break,
            };
            self.bytes_out = 0;
        }

        drop(ifd);

        if !self.to_stdout {
            self.copy_stat()?;

            if let Some(mut ofd) = self.ofd.take() {
                if let Err(err) = ofd.flush() {
                    eprintln!("{}: write error: {}", self.program_name, err);
                }
            }

            if !self.keep {
                if let Err(err) = fs::remove_file(path) {
                    eprintln!("{}: {}", self.program_name, err);
                }
            }
        }

        if self.method == -1 {
            if !self.to_stdout {
                self.remove_output_file()?;
            }
            return Ok(());
        }

        if self.verbose != 0 {
            if self.test {
                eprint!(" OK");
            } else if self.decompress {
                self.display_ratio(
                    self.bytes_out - (self.bytes_in as i64 - self.header_bytes as i64),
                    self.bytes_out,
                );
            } else {
                self.display_ratio(
                    self.bytes_in as i64 - (self.bytes_out - self.header_bytes as i64),
                    self.bytes_in as i64,
                );
            }
            if !self.test && !self.to_stdout {
                eprint!(" -- replaced with {}", self.ofname);
            }
            eprintln!();
        }
        return Ok(());
    }

    fn treat_dir(&mut self, dir: &Path) -> io::Result<()> {
        // Attempt to read the directory entries
        let dir_entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(_) => {
                self.progerror(dir);
                return Ok(());
            }
        };

        // Iterate over the directory entries
        for entry_result in dir_entries {
            let entry = match entry_result {
                Ok(e) => e,
                Err(_) => {
                    self.progerror(dir);
                    continue;
                }
            };

            let file_name = entry.file_name();
            let file_name_str = file_name.to_string_lossy();

            // Skip "." and ".." entries
            if file_name_str == "." || file_name_str == ".." {
                continue;
            }

            let dir_str = dir.to_string_lossy();
            let len = dir_str.len();
            let entrylen = file_name_str.len();

            // Check if the combined path length is within limits
            if len + entrylen < MAX_PATH_LEN - 2 {
                let mut nbuf = PathBuf::from(dir);

                // On some systems, an empty `dir` means the current directory
                if !dir_str.is_empty() {
                    nbuf.push(&file_name);
                } else {
                    nbuf = PathBuf::from(&file_name);
                }

                // Call treat_file with the new path
                if let Err(e) = self.treat_file(nbuf.to_str().unwrap()) {
                    eprintln!("Error processing file {}: {}", nbuf.display(), e);
                    self.exit_code = ERROR;
                }
            } else {
                eprintln!(
                    "{}: {}/{}: pathname too long",
                    self.program_name,
                    dir.display(),
                    file_name_str
                );
                self.exit_code = ERROR;
            }
        }

        Ok(())
    }

    fn treat_stdin(&mut self) -> io::Result<()> {
        if self.force == 0 && !self.list
            && (self.presume_input_tty || atty::is(if self.decompress { atty::Stream::Stdin } else { atty::Stream::Stdout })) {
            if !self.quiet {
                eprintln!(
                    "{}: compressed data not {} a terminal. Use -f to force {}compression.\nFor help, type: {} -h",
                    self.program_name,
                    if self.decompress { "read from" } else { "written to" },
                    if self.decompress { "de" } else { "" },
                    self.program_name
                );
            }
            self.do_exit(ERROR);
        }

        self.ifname = "stdin".to_string();
        self.ofname = "stdout".to_string();

        self.ifile_size = -1;

        if !self.no_time.unwrap_or(false) || self.list {
            self.time_stamp = Some(SystemTime::now());
        }

        self.clear_bufs();
        self.to_stdout = true;
        self.part_nb = 0;

        let mut stdin = io::stdin();

        if self.decompress {
            self.method = match self.get_method(&mut stdin)? {
                Some(method) => method,
                None => {
                    self.do_exit(self.exit_code);
                }
            };
        }

        if self.list {
            self.do_list(Some(&mut stdin), self.method)?;
            return Ok(());
        }

        loop {
            if let Some(work_fn) = self.work {
                self.ifd = Some(Box::new(io::stdin()));
                self.ofd = Some(Box::new(io::stdout()));
                if work_fn(self).is_err() {
                    return Ok(());
                }
            } else {
                eprintln!("{}: work function not set", self.program_name);
                return Ok(());
            }

            if self.input_eof()? {
                break;
            }

            self.method = match self.get_method(&mut stdin)? {
                Some(method) => method,
                None => return Ok(()),
            };
            self.bytes_out = 0;
        }

        if self.verbose != 0 {
            if self.test {
                eprintln!(" OK");
            } else if !self.decompress {
                self.display_ratio(
                    self.bytes_in as i64 - (self.bytes_out as i64 - self.header_bytes as i64),
                    self.bytes_in as i64,
                );
                eprintln!();
            }
        }

        Ok(())
    }

    fn get_method<R: Read>(&mut self, input: &mut R) -> io::Result<Option<i32>> {
        let flags: u8;
        let mut magic = [0u8; 10];
        let imagic0: Option<u8>;
        let imagic1: Option<u8>;
        let mut stamp: u32;

        if self.force == 0 && self.to_stdout {
            imagic0 = self.try_byte(input)?;
            if let Some(byte) = imagic0 {
                magic[0] = byte;
            }
            imagic1 = self.try_byte(input)?;
            if let Some(byte) = imagic1 {
                magic[1] = byte;
            }
        } else {
            magic[0] = self.get_byte(input)?;
            imagic0 = Some(0);
            if magic[0] != 0 {
                magic[1] = self.get_byte(input)?;
                imagic1 = Some(0);
            } else {
                imagic1 = self.try_byte(input)?;
                if let Some(byte) = imagic1 {
                    magic[1] = byte;
                }
            }
        }
        self.method = -1;
        self.part_nb += 1;
        self.header_bytes = 0;
        self.last_member = true;

        if magic[0..2] == GZIP_MAGIC[..] || magic[0..2] == OLD_GZIP_MAGIC[..] {
            self.method = self.get_byte(input)? as i32;
            if self.method != DEFLATED {
                eprintln!(
                    "{}: {}: unknown method {} -- not supported",
                    self.program_name, self.ifname, self.method
                );
                self.exit_code = ERROR;
                return Ok(None);
            }
            self.work = Some(unzip);
            flags = self.get_byte(input)?;

            if flags & ENCRYPTED != 0 {
                eprintln!(
                    "{}: {} is encrypted -- not supported",
                    self.program_name, self.ifname
                );
                self.exit_code = ERROR;
                return Ok(None);
            }
            if flags & RESERVED != 0 {
                eprintln!(
                    "{}: {} has flags 0x{:x} -- not supported",
                    self.program_name, self.ifname, flags
                );
                self.exit_code = ERROR;
                if self.force <= 1 {
                    return Ok(None);
                }
            }
            stamp = self.get_byte(input)? as u32;
            stamp |= (self.get_byte(input)? as u32) << 8;
            stamp |= (self.get_byte(input)? as u32) << 16;
            stamp |= (self.get_byte(input)? as u32) << 24;
            if stamp != 0 && !self.no_time.unwrap_or(false) {
                self.time_stamp = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(stamp as u64));
            }

            magic[8] = self.get_byte(input)?;
            magic[9] = self.get_byte(input)?;
            eprintln!("{:?}, magic", &magic[0..10]);
            if flags & HEADER_CRC != 0 {
//                 eprintln!("{:?}, magic", &magic[0..10]);
                magic[2] = DEFLATED as u8;
                magic[3] = flags;
                magic[4] = (stamp & 0xff) as u8;
                magic[5] = ((stamp >> 8) & 0xff) as u8;
                magic[6] = ((stamp >> 16) & 0xff) as u8;
                magic[7] = (stamp >> 24) as u8;
                self.updcrc(None, 0);
                self.updcrc(Some(&magic[0..10]), 10);
            }

            if flags & EXTRA_FIELD != 0 {
                let mut lenbuf = [0u8; 2];
                lenbuf[0] = self.get_byte(input)?;
                lenbuf[1] = self.get_byte(input)?;
                let len = lenbuf[0] as usize | ((lenbuf[1] as usize) << 8);
                if self.verbose != 0 {
                    eprintln!(
                        "{}: {}: extra field of {} bytes ignored",
                        self.program_name, self.ifname, len
                    );
                }
                if flags & HEADER_CRC != 0 {
                    self.updcrc(Some(&lenbuf), 2);
                }
                self.discard_input_bytes(input, len as usize, flags)?;
            }

            if flags & ORIG_NAME != 0 {
                if self.no_name.unwrap_or(false) || (self.to_stdout && !self.list) || self.part_nb > 1 {
                    self.discard_input_bytes(input, usize::MAX, flags)?;
                } else {
                    let p = self.ofname.clone();
                    let base = self.gzip_base_name(&p);
                    let mut p_bytes = base.as_bytes().to_vec();
                    loop {
                        let byte = self.get_byte(input)?;
                        p_bytes.push(byte);
                        if byte == 0 {
                            break;
                        }
                        if p_bytes.len() >= self.ofname.capacity() {
                            self.gzip_error("corrupted input -- file name too large");
                        }
                    }
                    if flags & HEADER_CRC != 0 {
                        self.updcrc(Some(&p_bytes), p_bytes.len());
                    }
                    let p_str = String::from_utf8_lossy(&p_bytes);
                    let new_base = self.gzip_base_name(&p_str);
                    self.ofname = new_base.to_string();
                    if !self.list {
                        self.make_legal_name();
                    }
                }
            }

            if flags & COMMENT != 0 {
                self.discard_input_bytes(input, usize::MAX, flags)?;
            }

            if flags & HEADER_CRC != 0 {
                let crc16 = self.updcrc(None, 0) & 0xffffffff;
                let mut header16 = self.get_byte(input)? as u32;
                header16 |= (self.get_byte(input)? as u32) << 8;
                eprintln!(
                        "{}: {}: header checksum 0x{:04x} != computed checksum 0x{:04x}",
                        self.program_name, self.ifname, header16, crc16
                    );
                if header16 != crc16 {
                    eprintln!(
                        "{}: {}: header checksum 0x{:04x} != computed checksum 0x{:04x}",
                        self.program_name, self.ifname, header16, crc16
                    );
                    self.exit_code = ERROR;
                    if self.force <= 1 {
                        return Ok(None);
                    }
                }
            }

            if self.part_nb == 1 {
                self.header_bytes = self.inptr + 2 * 4;
            }
            return Ok(Some(self.method));
        } else if magic[0..2] == PKZIP_MAGIC[..] && self.inptr == 2 && self.inbuf[0..4] == PKZIP_MAGIC[..] {
            self.inptr = 0;
            self.work = Some(unzip);
            if self.check_zipfile(input).is_err() {
                return Ok(None);
            }
            self.last_member = true;
            return Ok(Some(self.method));
        } else if magic[0..2] == PACK_MAGIC[..] {
            self.work = Some(unpack);
            self.method = PACKED as i32;
            return Ok(Some(self.method));
        } else if magic[0..2] == LZW_MAGIC[..] {
            self.work = Some(unlzw);
            self.method = COMPRESSED as i32;
            self.last_member = true;
            return Ok(Some(self.method));
        } else if magic[0..2] == LZH_MAGIC[..] {
            self.work = Some(unlzh);
            self.method = LZHED as i32;
            self.last_member = true;
            return Ok(Some(self.method));
        } else if self.force != 0 && self.to_stdout && !self.list {
            self.method = STORED as i32;
            self.work = Some(copy);
            if let Some(_byte) = imagic1 {
                self.inptr -= 1;
            }
            self.last_member = true;
            if let Some(byte) = imagic0 {
                self.write_buf(&mut io::stdout(), &[byte], 1)?;
                self.bytes_out += 1;
            }
            return Ok(Some(self.method));
        }

        if self.part_nb == 1 {
            eprintln!("\n{}: {}: not in gzip format", self.program_name, self.ifname);
            self.exit_code = ERROR;
            return Ok(None);
        } else {
            if magic[0] == 0 {
                let mut inbyte = imagic1;
                while inbyte == Some(0) {
                    inbyte = self.try_byte(input)?;
                }
                if inbyte.is_none() {
                    if self.verbose != 0 {
                        eprintln!(
                            "\n{}: {}: decompression OK, trailing zero bytes ignored",
                            self.program_name, self.ifname
                        );
                    }
                    return Ok(None);
                }
            }
            eprintln!(
                "\n{}: {}: decompression OK, trailing garbage ignored",
                self.program_name, self.ifname
            );
            return Ok(None);
        }
    }

    fn get_byte<R: Read>(&mut self, input: &mut R) -> io::Result<u8> {
        if self.inptr >= self.insize {
            self.insize = input.read(&mut self.inbuf)?;
            self.bytes_in += self.insize as i64;
            self.inptr = 0;
            if self.insize == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "Unexpected EOF"));
            }
        }
        let byte = self.inbuf[self.inptr];
        self.inptr += 1;
        Ok(byte)
    }

    fn try_byte<R: Read>(&mut self, input: &mut R) -> io::Result<Option<u8>> {
        if self.inptr >= self.insize {
            self.insize = input.read(&mut self.inbuf)?;
            self.bytes_in += self.insize as i64;
            self.inptr = 0;
            if self.insize == 0 {
                return Ok(None);
            }
        }
        let byte = self.inbuf[self.inptr];
        self.inptr += 1;
        Ok(Some(byte))
    }

    fn discard_input_bytes<R: Read>(&mut self, input: &mut R, mut nbytes: usize, flags: u8) -> io::Result<()> {
        if nbytes != usize::MAX {
            while nbytes != 0 {
                let c = self.get_byte(input)?;
                if flags & HEADER_CRC != 0 {
                    self.updcrc(Some(&[c]), 1);
                }
                nbytes -= 1;
            }
        } else {
            loop {
                let c = self.get_byte(input)?;
                if flags & HEADER_CRC != 0 {
                    self.updcrc(Some(&[c]), 1);
                }
                if c == 0 {
                    break;
                }
            }
        }
        Ok(())
    }

//     fn updcrc(&mut self, buf: Option<&[u8]>, len: usize) -> u32 {
//         if buf.is_none() {
//             self.crc16_digest = CRC16.digest();
//         } else if let Some(data) = buf {
//             self.crc16_digest.update(&data[..len]);
//         }
//         self.crc16_digest.clone().finalize() as u32
//     }

    fn updcrc(&mut self, buf: Option<&[u8]>, len: usize) -> u32 {
        if buf.is_none() {
            self.crc16_digest = 0xffffffff;
        } else {
            self.crc16_digest = self.crc16_digest; // 使用当前的 crc16_digest
            let bytes = buf.unwrap();
            let mut i = 0;
            while i < len {
                self.crc16_digest = CRC_32_TAB[((self.crc16_digest ^ bytes[i] as u32) & 0xff) as usize] ^ (self.crc16_digest >> 8);
                i += 1;
            }
        }
        let count = COUNT.fetch_add(1, Ordering::SeqCst);
        self.crc16_digest ^ 0xffffffff // 返回最终的 CRC 值
    }

    fn gzip_base_name<'a>(&self, fname: &'a str) -> &'a str {
        Path::new(fname)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(fname)
    }

    fn gzip_error(&self, msg: &str) -> ! {
        if !self.ifname.is_empty() {
            eprintln!("{}: {}: {}", self.program_name, self.ifname, msg);
        } else {
            eprintln!("{}: {}", self.program_name, msg);
        }
        self.do_exit(ERROR);
    }

    fn make_legal_name(&mut self) {
        use std::path::Path;

        // Extract the file name without any directory components
        if let Some(file_name) = Path::new(&self.ofname).file_name() {
            self.ofname = file_name.to_string_lossy().into_owned();
        }

        // Replace any invalid characters in the file name
        let invalid_chars = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];
        let mut legal_name = String::new();
        for c in self.ofname.chars() {
            if invalid_chars.contains(&c) {
                legal_name.push('_');
            } else {
                legal_name.push(c);
            }
        }
        self.ofname = legal_name;
    }

    fn write_buf<W: Write>(&mut self, output: &mut W, buf: &[u8], count: usize) -> io::Result<()> {
        output.write_all(&buf[..count])
    }

    fn check_zipfile<R: Read>(&mut self, input: &mut R) -> io::Result<()> {
        const ZIP_LOCAL_HEADER_SIGNATURE: u32 = 0x04034b50;

        // Read the local file header
        let signature = self.read_u32_le(input)?;
        if signature != ZIP_LOCAL_HEADER_SIGNATURE {
            eprintln!("{}: {}: not a valid zip file", self.program_name, self.ifname);
            self.exit_code = ERROR;
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid ZIP file"));
        }

        // Read and parse the local file header
        let _version_needed = self.read_u16_le(input)?;
        let _flags = self.read_u16_le(input)?;
        let compression_method = self.read_u16_le(input)?;
        let _last_mod_time = self.read_u16_le(input)?;
        let _last_mod_date = self.read_u16_le(input)?;
        let _crc32 = self.read_u32_le(input)?;
        let _compressed_size = self.read_u32_le(input)?;
        let _uncompressed_size = self.read_u32_le(input)?;
        let file_name_length = self.read_u16_le(input)? as usize;
        let extra_field_length = self.read_u16_le(input)? as usize;

        // Read the file name
        let mut file_name_bytes = vec![0u8; file_name_length];
        input.read_exact(&mut file_name_bytes)?;
        let file_name = String::from_utf8_lossy(&file_name_bytes);

        // Set the output file name if necessary
        if !self.no_name.unwrap_or(false) {
            self.ofname = file_name.to_string();
            if !self.list {
                self.make_legal_name();
            }
        }

        // Skip the extra field
        let mut extra_field = vec![0u8; extra_field_length];
        input.read_exact(&mut extra_field)?;

        // Set the compression method
        self.method = compression_method as i32;
        self.last_member = true; // Assume single member for simplicity

        // Prepare the work function based on the compression method
        match self.method {
            0 => self.work = Some(copy),          // Stored (no compression)
            8 => self.work = Some(unzip),         // Deflated
            12 => self.work = Some(unlzh),        // BZIP2 (if supported)
            _ => {
                eprintln!(
                    "{}: {}: unsupported compression method {} in zip file",
                    self.program_name, self.ifname, self.method
                );
                self.exit_code = ERROR;
                return Err(io::Error::new(io::ErrorKind::InvalidData, "Unsupported compression method"));
            }
        }

        Ok(())
    }

    fn read_u16_le<R: Read>(&mut self, input: &mut R) -> io::Result<u16> {
        input.read_u16::<LittleEndian>()
    }

    fn read_u32_le<R: Read>(&mut self, input: &mut R) -> io::Result<u32> {
        input.read_u32::<LittleEndian>()
    }

    fn do_list<R: Read>(&mut self, input: Option<&mut R>, method: i32) -> io::Result<()> {
        const METHODS: [&str; MAX_METHODS] = [
            "store",  /* 0 */
            "compr",  /* 1 */
            "pack ",  /* 2 */
            "lzh  ",  /* 3 */
            "", "", "", "", /* 4 to 7 reserved */
            "defla",  /* 8 */
        ];

        let mut positive_off_t_width = 1;
        let mut o = i64::MAX;

        while o > 9 {
            positive_off_t_width += 1;
            o /= 10;
        }

        if self.first_time && method >= 0 {
            self.first_time = false;
            if self.verbose != 0 {
                print!("method  crc     date  time  ");
            }
            if !self.quiet {
                println!(
                    "{:>width$} {:>width$}  ratio uncompressed_name",
                    "compressed",
                    "uncompressed",
                    width = positive_off_t_width as usize
                );
            }
        } else if method < 0 {
            if self.total_in <= 0 || self.total_out <= 0 {
                return Ok(());
            }
            if self.verbose != 0 {
                print!("                            ");
            }
            if self.verbose != 0 || !self.quiet {
                self.fprint_off(&mut stdout(), self.total_in, positive_off_t_width)?;
                print!(" ");
                self.fprint_off(&mut stdout(), self.total_out, positive_off_t_width)?;
                print!(" ");
            }
            self.display_ratio(
                self.total_out - (self.total_in - self.header_bytes as i64),
                self.total_out,
            );
            println!(" (totals)");
            return Ok(());
        }

        let mut crc: u32 = !0; // unknown
        self.bytes_out = -1;
        self.bytes_in = self.ifile_size.try_into().unwrap();

        if !self.record_io && method == DEFLATED && !self.last_member {
            // Get the crc and uncompressed size for gzip'ed (not zip'ed) files.
            // If the seek fails, we could use read() to get to the end, but
            // --list is used to get quick results.
            // Use "gunzip < foo.gz | wc -c" to get the uncompressed size if
            // you are not concerned about speed.
            //self.bytes_in = input.seek(SeekFrom::End(-8))? as i64;
            //if self.bytes_in != -1 {
                let mut buf = [0u8; 8];
                input.unwrap().read_exact(&mut buf)?;
                self.bytes_in += 8;
                crc = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
                self.bytes_out = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]) as i64;
            //}
        }

        if self.verbose != 0 {
            print!("{:5} {:08x} ", METHODS[method as usize], crc);
            if let Some(time_stamp) = self.time_stamp {
                let datetime: DateTime<Local> = DateTime::from(time_stamp);
                print!(
                    "{}{:3} {:02}:{:02} ",
                    datetime.format("%b"),
                    datetime.day(),
                    datetime.hour(),
                    datetime.minute()
                );
            } else {
                print!("??? ?? ??:?? ");
            }
        }

        self.fprint_off(&mut stdout(), self.bytes_in as i64, positive_off_t_width)?;
        print!(" ");
        self.fprint_off(&mut stdout(), self.bytes_out, positive_off_t_width)?;
        print!(" ");

        if self.bytes_in == -1 {
            self.total_in = -1;
            self.bytes_in = 0;
            self.bytes_out = 0;
            self.header_bytes = 0;
        } else if self.total_in >= 0 {
            self.total_in += self.bytes_in as i64;
        }

        if self.bytes_out == -1 {
            self.total_out = -1;
            self.bytes_in = 0;
            self.bytes_out = 0;
            self.header_bytes = 0;
        } else if self.total_out >= 0 {
            self.total_out += self.bytes_out;
        }

        self.display_ratio(
            self.bytes_out - (self.bytes_in as i64 - self.header_bytes as i64),
            self.bytes_out,
        );
        println!(" {}", self.ofname);

        Ok(())
    }

    fn fprint_off<W: Write>(&self, file: &mut W, mut offset: i64, width: usize) -> io::Result<()> {
        // Buffer to hold the string representation of the offset
        let mut buf = [0u8; 65]; // 64 digits max for i64 plus sign
        let mut p = buf.len();

        // Don't negate offset here; it might overflow.
        if offset < 0 {
            // Build the digits in reverse order
            loop {
                p -= 1;
                buf[p] = b'0' - (offset % 10) as u8;
                offset /= 10;
                if offset == 0 {
                    break;
                }
            }
            p -= 1;
            buf[p] = b'-';
        } else {
            // Positive offset
            loop {
                p -= 1;
                buf[p] = b'0' + (offset % 10) as u8;
                offset /= 10;
                if offset == 0 {
                    break;
                }
            }
        }

        // Calculate the number of digits
        let num_digits = buf.len() - p;

        // Adjust the width by subtracting the number of digits
        let mut width = if width > num_digits {
            width - num_digits
        } else {
            0
        };

        // Write leading spaces to align the number to the right
        while width > 0 {
            file.write_all(b" ")?;
            width -= 1;
        }

        // Write the number to the file
        file.write_all(&buf[p..])?;
        Ok(())
    }

    // Function to install signal handlers
    fn install_signal_handlers(&self) {
        // Implement signal handling if necessary
    }

    fn make_ofname(&mut self) -> io::Result<()> {
        if self.to_stdout {
            // Output is stdout; no need to modify ofname
            return Ok(());
        }

        self.ofname = self.ifname.clone();

        if self.decompress {
            // Decompressing: remove the suffix
            if self.z_len == 0 {
                eprintln!("{}: no suffix specified", self.program_name);
                self.exit_code = ERROR;
                return Err(io::Error::new(io::ErrorKind::Other, "no suffix specified"));
            }

            if self.ofname.len() >= self.z_len && self.ofname.ends_with(&self.z_suffix) {
                // Remove the suffix
                let new_len = self.ofname.len() - self.z_len;
                self.ofname.truncate(new_len);
            } else {
                // Input file does not have the expected suffix
                if self.force == 0 && !self.list && !self.test {
                    eprintln!(
                        "{}: {}: unknown suffix -- ignored",
                        self.program_name, self.ifname
                    );
                    self.exit_code = ERROR;
                    return Err(io::Error::new(io::ErrorKind::Other, "unknown suffix"));
                }
                if !self.to_stdout {
                    self.ofname = self.ifname.clone();
                }
            }
        } else {
            // Compressing: append the suffix
            self.ofname.push_str(&self.z_suffix);
        }

        Ok(())
    }

    fn create_outfile(&self) -> io::Result<File> {
        use std::fs::OpenOptions;
        let mut options = OpenOptions::new();
        options.write(true);

        if self.force == 1 {
            options.create(true).truncate(true);
        } else {
            options.create_new(true);
        }

        let file = options.open(&self.ofname)?;
        Ok(file)
    }

    fn copy_stat(&self) -> io::Result<()> {
        // Copy file metadata from input to output
        // For simplicity, we'll set the modified time
        if let Some(ref time_stamp) = self.time_stamp {
            let result = filetime::set_file_mtime(&self.ofname, filetime::FileTime::from_system_time(*time_stamp));
            if let Err(err) = result {
                eprintln!("{}: {}", self.program_name, err);
            }
        }
        Ok(())
    }

    fn remove_output_file(&self) -> io::Result<()> {
        fs::remove_file(&self.ofname)?;
        Ok(())
    }

    fn input_eof(&self) -> io::Result<bool> {
        // Implement logic to check if input EOF is reached
        // For simplicity, return true to end the loop
        Ok(true)
    }

    fn display_ratio(&self, num: i64, den: i64) {
        if den == 0 {
            print!("inf%");
        } else {
            let ratio = 100.0 * num as f64 / den as f64;
            print!("{:.2}%", ratio);
        }
    }

    fn clear_bufs(&mut self) {
        // Clear any buffers if needed
        self.bytes_in = 0;
        self.bytes_out = 0;
        self.insize = 0;
        self.inptr = 0;
        self.outcnt = 0;
    }

    // Function to write a single byte
    fn put_byte(&mut self, mut byte: u8) -> io::Result<()> {
        if self.to_stdout && String::from_utf8_lossy(&[byte]).as_bytes()[0] != byte {
            self.ofd.as_mut().unwrap().write_all(String::from_utf8_lossy(&[byte]).as_bytes())?;
            self.outcnt += 1;
            return Ok(());
        }
        self.ofd.as_mut().unwrap().write_all(&[byte])?;
        self.outcnt += 1;
//         self.crc16_digest = self.updcrc(Some(&[byte]), 1);
        Ok(())
    }

    /// Send a value on a given number of bits.
    /// IN assertion: length <= 16 and value fits in length bits.
    fn send_bits(&mut self, mut value: u16, length: u8) {
        // If not enough room in bi_buf, use (valid) bits from bi_buf and
        // (16 - bi_valid) bits from value, leaving (width - (16 - bi_valid))
        // unused bits in value.

        const BUF_SIZE: u8 = 16; // Size of bi_buf in bits

        if self.bi_valid + length >= BUF_SIZE {
            // bi_buf has less room than the number of bits we need to add
            self.bi_buf |= value << self.bi_valid;
//             println!("Current bi_buf (before put_short): {:04x}", self.bi_buf);
            self.put_short(self.bi_buf);

            // Shift the value right by (BUF_SIZE - bi_valid) bits
            self.bi_buf = ((value as u32) >> (BUF_SIZE - self.bi_valid)) as u16;
            self.bi_valid = self.bi_valid + length - BUF_SIZE;
        } else {
            // There is enough room in bi_buf
            self.bi_buf |= value << self.bi_valid;
            self.bi_valid += length;
        }
    }

    fn put_short(&mut self, value: u16) {
        self.put_byte((value & 0xFF) as u8).unwrap();        // Lower byte
        self.put_byte(((value >> 8) & 0xFF) as u8).unwrap(); // Upper byte
    }

    fn bi_windup(&mut self) {
        if self.bi_valid > 8 {
            self.put_short(self.bi_buf);
        } else if self.bi_valid > 0 {
            self.put_byte(self.bi_buf as u8).expect("Failed!");
        }
        self.bi_buf = 0;
        self.bi_valid = 0;
    }

    // Function to write a 4-byte little-endian unsigned long
    fn put_long(&mut self, value: u32) -> io::Result<()> {
        let bytes = value.to_le_bytes();
        self.ofd.as_mut().unwrap().write_all(&bytes)?;
        self.outcnt += 4;
//         self.crc16_digest = self.updcrc(Some(&bytes), bytes.len());
        Ok(())
    }
}

fn unpack(_state: &mut GzipState) -> io::Result<()> {
    unimplemented!()
}

fn unlzw(_state: &mut GzipState) -> io::Result<()> {
    unimplemented!()
}

fn lzw(_state: &mut GzipState) -> io::Result<()> {
    unimplemented!()
}

fn unlzh(_state: &mut GzipState) -> io::Result<()> {
    unimplemented!()
}

// fn unzip(_state: &mut GzipState) -> io::Result<()> {
//     unimplemented!()
// }

fn copy(_state: &mut GzipState) -> io::Result<()> {
    unimplemented!()
}

// CRC calculation functions
fn crc32d(c: u32, data: u64) -> u32 {
    // Implement CRC32D logic (using the equivalent algorithm)
    let mut crc = c;
    for i in 0..8 {
        crc = crc32b(crc, (data >> (i * 8)) as u8);
    }
    crc
}

fn crc32w(c: u32, data: u32) -> u32 {
    // Implement CRC32W logic (using the equivalent algorithm)
    crc32d(c, data as u64)
}

fn crc32h(c: u32, data: u16) -> u32 {
    // Implement CRC32H logic (using the equivalent algorithm)
    crc32d(c, data as u64)
}

fn crc32b(c: u32, data: u8) -> u32 {
    // Implement CRC32B logic (using the equivalent algorithm)
    let poly: u32 = 0xedb88320;
    let mut crc = c ^ 0xff; // Initial value for CRC
    crc ^= data as u32;

    for _ in 0..8 {
        if (crc & 1) != 0 {
            crc = (crc >> 1) ^ poly;
        } else {
            crc >>= 1;
        }
    }
    crc
}

fn main() -> io::Result<()> {
    let mut state = GzipState::new();
    
    // Parse command-line arguments
    state.parse_args();

    // Run the main processing loop
    state.run()
}