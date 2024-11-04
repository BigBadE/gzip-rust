use std::io;
use std::time::SystemTime;
use crate::{GzipState, DEFLATED, GZIP_MAGIC, ORIG_NAME, OS_CODE};
use crate::deflate::Deflate;
use crate::trees::Trees;

pub fn zip (state: &mut GzipState) -> io::Result<()> {
    // Initialize output count
    state.outcnt = 0;

    // Write the gzip header
    state.method = DEFLATED;
    state.put_byte(GZIP_MAGIC[0])?;
    state.put_byte(GZIP_MAGIC[1])?;
    state.put_byte(DEFLATED as u8)?;
    let mut flags = 0;

    if state.save_orig_name {
        flags |= ORIG_NAME;
    }
    state.put_byte(flags)?;         // general flags

    let stamp = if let Some(time_stamp) = state.time_stamp {
        match time_stamp.duration_since(SystemTime::UNIX_EPOCH) {
            Ok(duration) => {
                let secs = duration.as_secs();
                if secs <= u32::MAX as u64 {
                    secs as u32
                } else {
                    0
                }
            }
            Err(_) => 0,
        }
    } else {
        0
    };

    state.put_long(stamp)?;

    // Initialize CRC
    state.updcrc(None, 0);

    // Initialize compression (bi_init, ct_init, lm_init)
    let mut trees = Trees::new();
    let mut deflate = Deflate::new();
    let mut attr = 0;
    let mut deflate_flags = 0;
    trees.ct_init(&mut attr, state.method);
    deflate.lm_init(state, state.level, &mut deflate_flags);

    // Write deflate flags and OS identifier
    state.put_byte(deflate_flags as u8)?; // Assuming `deflate_flags` fits in u8
    state.put_byte(OS_CODE)?;

    // Write original filename if `save_orig_name` is set
    if state.save_orig_name {
        let basename = state.gzip_base_name(&state.ifname).to_string();
        for byte in basename.bytes() {
            state.put_byte(byte)?;
        }
        state.put_byte(0)?; // Null-terminate the filename
    }

    // Record header bytes
    state.header_bytes = state.outcnt;

    // Perform deflation (compression)
    deflate.deflate(&mut trees, state)?;

    // Optionally check input size (similar to C code)
    #[cfg(not(any(target_os = "windows", target_os = "vms")))]
    {
        if state.ifile_size != -1 && state.bytes_in != state.ifile_size {
            eprintln!(
                "{}: {}: file size changed while zipping",
                state.program_name, state.ifname
            );
        }
    }

    // Write the CRC and uncompressed size
    let crc_value = state.crc16_digest;
    let uncompressed_size = state.bytes_in.try_into().unwrap();

    println!("CRC: {:x}", crc_value);
    println!("Uncompressed Size: {:x}", uncompressed_size);

    state.put_long(crc_value)?;
    state.put_long(uncompressed_size)?;


    state.header_bytes += 8; // 2 * 4 bytes

    Ok(())
}