use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::{cmp, env};

use anyhow::{Error, Result};
use xmas_elf::ElfFile;

pub const VAR_BIN_FILE: &str = "EMBUILD_GENERATED_BIN_FILE";

pub fn run(elf: impl AsRef<Path>) -> Result<()> {
    let output_file = PathBuf::from(env::var("OUT_DIR")?).join("binary.bin");

    run_for_file(elf, &output_file)?;

    println!("cargo:rustc-env={}={}", VAR_BIN_FILE, output_file.display());

    Ok(())
}

pub fn run_for_file(elf: impl AsRef<Path>, output_file: impl AsRef<Path>) -> Result<()> {
    let output_file = output_file.as_ref();

    eprintln!("Output: {:?}", output_file);

    write(elf, &mut File::create(output_file)?)
}

pub fn write(elf: impl AsRef<Path>, output: &mut impl Write) -> Result<()> {
    eprintln!("Input: {:?}", elf.as_ref());

    let elf_data = fs::read(elf.as_ref())?;
    let elf = ElfFile::new(&elf_data).map_err(Error::msg)?;

    let mut sorted = segments::segments(&elf).collect::<Vec<_>>();
    sorted.sort();

    let mut offset: u64 = 0;
    for segment in sorted {
        let buf = [0_u8; 4096];
        while offset < segment.addr {
            let delta = cmp::min(buf.len() as u64, segment.addr - offset) as usize;

            output.write_all(&buf[0..delta])?;

            offset += delta as u64;
        }

        output.write_all(segment.data)?;
        offset += segment.data.len() as u64;
    }

    Ok(())
}

mod segments {
    use std::cmp::Ordering;

    use xmas_elf::program::{SegmentData, Type};
    use xmas_elf::ElfFile;

    /// A segment of code from the source elf
    #[derive(Debug, Ord, Eq)]
    pub struct CodeSegment<'a> {
        pub addr: u64,
        pub size: u64,
        pub data: &'a [u8],
    }

    impl PartialEq for CodeSegment<'_> {
        fn eq(&self, other: &Self) -> bool {
            self.addr.eq(&other.addr)
        }
    }

    impl PartialOrd for CodeSegment<'_> {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            self.addr.partial_cmp(&other.addr)
        }
    }

    pub fn segments<'a>(elf: &'a ElfFile<'a>) -> impl Iterator<Item = CodeSegment<'a>> + 'a {
        elf.program_iter()
            .filter(|header| {
                header.file_size() > 0 && header.get_type() == Ok(Type::Load) && header.offset() > 0
            })
            .flat_map(move |header| {
                let addr = header.virtual_addr();
                let size = header.file_size();
                let data = match header.get_data(elf) {
                    Ok(SegmentData::Undefined(data)) => data,
                    _ => return None,
                };
                Some(CodeSegment { addr, data, size })
            })
    }
}
