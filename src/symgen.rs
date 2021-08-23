use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::{env, fmt};

use anyhow::*;
use xmas_elf::sections::{SectionData, ShType};
use xmas_elf::{symbol_table, ElfFile};

pub const VAR_SYMBOLS_FILE: &str = "EMBUILD_GENERATED_SYMBOLS_FILE";

pub fn run(elf: impl AsRef<Path>, start_addr: u64) -> Result<()> {
    let output_file = PathBuf::from(env::var("OUT_DIR")?).join("symbols.rs");

    run_for_file(elf, start_addr, &output_file)?;

    println!(
        "cargo:rustc-env={}={}",
        VAR_SYMBOLS_FILE,
        output_file.display()
    );

    Ok(())
}

pub fn run_for_file(
    elf: impl AsRef<Path>,
    start_addr: u64,
    output_file: impl AsRef<Path>,
) -> Result<()> {
    let output_file = output_file.as_ref();

    eprintln!("Output: {:?}", output_file);

    write(elf, start_addr, &mut File::create(output_file)?)
}

pub fn write(elf: impl AsRef<Path>, start_addr: u64, output: &mut impl Write) -> Result<()> {
    eprintln!("Input: {:?}", elf.as_ref());

    let elf_data = fs::read(elf.as_ref())?;
    let elf = ElfFile::new(&elf_data).map_err(Error::msg)?;

    for symtable in get_symtables(&elf) {
        match symtable {
            SectionData::SymbolTable32(entries) => {
                write_symbols(&elf, start_addr, entries.iter(), output)?
            }
            SectionData::SymbolTable64(entries) => {
                write_symbols(&elf, start_addr, entries.iter(), output)?
            }
            _ => panic!(),
        }
    }

    Ok(())
}

fn write_symbols<'a, W: Write>(
    elf: &ElfFile<'a>,
    start_addr: u64,
    symbols: impl Iterator<Item = &'a (impl symbol_table::Entry + fmt::Debug + 'a)>,
    output: &mut W,
) -> Result<()> {
    for sym in symbols {
        eprintln!("Found symbol: {:?}", sym);

        if sym.get_type().map_err(Error::msg)? == symbol_table::Type::Object {
            let name = sym.get_name(elf).map_err(Error::msg)?;
            if !name.contains('.') {
                eprintln!("Writing symbol: {:?}", sym);
                write!(
                    output,
                    "#[allow(dead_code)]\npub const {name}: *{mut} core::ffi::c_void = 0x{addr:x} as *{mut} core::ffi::c_void;\n",
                    name = name,
                    mut = "mut", // TODO
                    addr = start_addr + sym.value()
                )?;
            } else {
                eprintln!("Skipping symbol: {:?}", sym);
            }
        }
    }

    Ok(())
}

fn get_symtables<'a, 'b>(elf: &'b ElfFile<'a>) -> impl Iterator<Item = SectionData<'a>> + 'b {
    elf.section_iter()
        .filter(|header| header.get_type().map_err(Error::msg).unwrap() == ShType::SymTab)
        .map(move |header| header.get_data(elf).unwrap())
}
