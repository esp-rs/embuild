use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::{env, fmt};

use anyhow::{Error, Result};
use xmas_elf::sections::{SectionData, ShType};
use xmas_elf::symbol_table::{Binding, Visibility};
use xmas_elf::{symbol_table, ElfFile};

pub const VAR_SYMBOLS_FILE: &str = "EMBUILD_GENERATED_SYMBOLS_FILE";

#[derive(Debug)]
pub struct Symbol<'a> {
    name: &'a str,
    section_name: Option<&'a str>,
    visible: bool,
    global: bool,
}

#[derive(Debug)]
pub struct Section {
    pub name: String,
    pub prefix: Option<String>,
    pub mutable: bool,
}

impl Section {
    pub fn new(name: impl Into<String>, prefix: Option<String>, mutable: bool) -> Self {
        Self {
            name: name.into(),
            prefix,
            mutable,
        }
    }

    pub fn code(name: impl Into<String>) -> Self {
        Self::new(name, None, false)
    }

    pub fn data(name: impl Into<String>) -> Self {
        Self::new(name, None, true)
    }
}

impl<'a> Symbol<'a> {
    /// Get a reference to the symbol's name.
    pub fn name(&self) -> &'a str {
        self.name
    }

    /// Get a reference to the symbol's section name.
    pub fn section_name(&self) -> Option<&'a str> {
        self.section_name
    }

    /// Get a reference to the symbol's visible.
    pub fn visible(&self) -> bool {
        self.visible
    }

    /// Get a reference to the symbol's global.
    pub fn global(&self) -> bool {
        self.global
    }

    pub fn default_pointer_gen(&self) -> Option<RustPointer> {
        if self.section_name().is_some() && self.global() && self.visible() {
            let valid_identifier = self.name().char_indices().all(|(index, ch)| {
                ch == '_' || index == 0 && ch.is_alphabetic() || index > 0 && ch.is_alphanumeric()
            });

            if valid_identifier {
                return Some(RustPointer {
                    name: self.name().to_owned(),
                    mutable: true,
                    r#type: None,
                });
            }
        }

        None
    }

    pub fn default_sections(&self) -> Option<RustPointer> {
        self.sections(&[Section::data(".bss"), Section::data(".data")])
    }

    pub fn sections<'b>(
        &'b self,
        sections: impl IntoIterator<Item = &'b Section>,
    ) -> Option<RustPointer> {
        self.default_pointer_gen().and_then(move |mut pointer| {
            sections
                .into_iter()
                .find(|section| self.section_name() == Some(&section.name))
                .map(|section| {
                    if let Some(section_prefix) = &section.prefix {
                        pointer.name = format!("{}{}", section_prefix, pointer.name);
                    }

                    pointer
                })
        })
    }
}

#[derive(Debug, Clone)]
pub struct RustPointer {
    pub name: String,
    pub mutable: bool,
    pub r#type: Option<String>,
}

#[allow(clippy::type_complexity)]
pub struct Symgen {
    elf: PathBuf,
    start_addr: u64,
    rust_pointer_gen: Box<dyn for<'a> Fn(&Symbol<'a>) -> Option<RustPointer>>,
}

impl Symgen {
    pub fn new(elf: impl Into<PathBuf>, start_addr: u64) -> Self {
        Self::new_with_pointer_gen(elf, start_addr, |symbol| symbol.default_sections())
    }

    pub fn new_with_pointer_gen(
        elf: impl Into<PathBuf>,
        start_addr: u64,
        rust_pointer_gen: impl for<'a> Fn(&Symbol<'a>) -> Option<RustPointer> + 'static,
    ) -> Self {
        Self {
            elf: elf.into(),
            start_addr,
            rust_pointer_gen: Box::new(rust_pointer_gen),
        }
    }

    pub fn run(&self) -> Result<PathBuf> {
        let output_file = PathBuf::from(env::var("OUT_DIR")?).join("symbols.rs");

        self.run_for_file(&output_file)?;

        println!(
            "cargo:rustc-env={}={}",
            VAR_SYMBOLS_FILE,
            output_file.display()
        );

        Ok(output_file)
    }

    pub fn run_for_file(&self, output_file: impl AsRef<Path>) -> Result<()> {
        let output_file = output_file.as_ref();

        eprintln!("Output: {:?}", output_file);

        self.write(&mut File::create(output_file)?)
    }

    pub fn write(&self, output: &mut impl Write) -> Result<()> {
        eprintln!("Input: {:?}", self.elf);

        let elf_data = fs::read(&self.elf)?;
        let elf = ElfFile::new(&elf_data).map_err(Error::msg)?;

        for symtable in self.get_symtables(&elf) {
            match symtable.1 {
                SectionData::SymbolTable32(entries) => {
                    self.write_symbols(&elf, symtable.0, entries.iter().enumerate(), output)?
                }
                SectionData::SymbolTable64(entries) => {
                    self.write_symbols(&elf, symtable.0, entries.iter().enumerate(), output)?
                }
                _ => unimplemented!(),
            }
        }

        Ok(())
    }

    fn write_symbols<'a, W: Write>(
        &self,
        elf: &'a ElfFile<'a>,
        symtable_index: usize,
        symbols: impl Iterator<Item = (usize, &'a (impl symbol_table::Entry + fmt::Debug + 'a))>,
        output: &mut W,
    ) -> Result<()> {
        for (_index, sym) in symbols {
            eprintln!("Found symbol: {:?}", sym);

            let sym_type = sym.get_type().map_err(Error::msg)?;

            if sym_type == symbol_table::Type::Object || sym_type == symbol_table::Type::NoType {
                let name = sym.get_name(elf).map_err(Error::msg)?;

                let section_name = sym
                    .get_section_header(elf, symtable_index)
                    .and_then(|sh| sh.get_name(elf))
                    .ok();

                let global = sym.get_binding().map_err(Error::msg)? == Binding::Global;
                let visible = matches!(sym.get_other(), Visibility::Default);

                let symbol = Symbol {
                    name,
                    section_name,
                    global,
                    visible,
                };

                let pointer = (self.rust_pointer_gen)(&symbol);

                if let Some(pointer) = pointer {
                    eprintln!("Writing symbol: {} [{:?}] as [{:?}]", name, symbol, pointer);
                    write!(
                        output,
                        "#[allow(dead_code, non_upper_case_globals)]\npub const {name}: *{mutable} {typ} = 0x{addr:x} as *{mutable} {typ};\n",
                        name = pointer.name,
                        mutable = if pointer.mutable { "mut" } else {"const" },
                        typ = pointer.r#type.unwrap_or_else(|| "core::ffi::c_void".to_owned()),
                        addr = self.start_addr + sym.value()
                    )?;
                } else {
                    eprintln!("Skipping symbol: {} [{:?}]", name, sym);
                }
            }
        }

        Ok(())
    }

    fn get_symtables<'a, 'b>(
        &self,
        elf: &'b ElfFile<'a>,
    ) -> impl Iterator<Item = (usize, SectionData<'a>)> + 'b {
        elf.section_iter()
            .enumerate()
            .filter(|(_, header)| header.get_type().map_err(Error::msg).unwrap() == ShType::SymTab)
            .map(move |(index, header)| (index, header.get_data(elf).unwrap()))
    }
}
