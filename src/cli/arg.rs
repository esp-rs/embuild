use std::fmt::Display;

use bitflags::bitflags;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Arg {
    /// A flag with a name (ex. `-n` or `--name`).
    Flag,
    /// An option with a `name` and `value`
    ///
    /// Could be parsed depending on [`ArgOpts`] as:
    /// - `--name=value`
    /// - `--name value`
    /// - `--namevalue`
    /// - `-nvalue`
    /// - `-namevalue`
    /// - `-name value`
    /// - `-name=value`
    ///
    /// Is serialized as (default):
    /// - `-n value` if `name` is a single character,
    /// - `--name value` otherwise.
    Option,
}

impl Arg {
    /// Create an [`ArgDef`] from a `Arg::Flag` with `name`.
    pub const fn flag(name: &str) -> ArgDef<'_, 'static> {
        Self::Flag.with_name(name)
    }

    /// Create an [`ArgDef`] from an `Arg::Option` with `name`.
    pub const fn option(name: &str) -> ArgDef<'_, 'static> {
        Self::Option.with_name(name)
    }

    /// Create an [`ArgDef`] from this `Arg` with `name`.
    pub const fn with_name<'a>(self, name: &'a str) -> ArgDef<'a, 'static> {
        ArgDef {
            arg: self,
            name,
            alias: &[],
            opts: ArgOpts::empty(),
        }
    }
}

bitflags! {
    pub struct ArgOpts: u32 {
        /// The argument can use a single hypen (ex. `-<argname>`)
        const SINGLE_HYPHEN = (1 << 0);
        /// The argument can use two hyphens (ex. `--<argname>`)
        const DOUBLE_HYPHEN = (1 << 1);
        /// The argument can have whitespace to seperate the value (ex. `--<argname> <value>`)
        const VALUE_SEP_NEXT_ARG = (1 << 2);
        /// The argument can have an equals (`=`) to seperate the value (ex.
        /// `--<argname>=<value>`)
        const VALUE_SEP_EQUALS = (1 << 3);
        /// The argument can have no seperator for the value (ex. `--<argname><value>`)
        ///
        /// Note: This will also match [`VALUE_SEP_EQUALS`](ArgOpts::VALUE_SEP_EQUALS) but
        /// keep the equals sign in the value: `--<argument>=<value>` -> `Some("=<value>")`.
        const VALUE_SEP_NO_SPACE = (1 << 4);
        /// The argument's value is optional
        const VALUE_OPTIONAL = (1 << 5);

        const ALL_HYPHEN = Self::SINGLE_HYPHEN.bits | Self::DOUBLE_HYPHEN.bits;
        const ALL_VALUE_SEP = Self::VALUE_SEP_EQUALS.bits | Self::VALUE_SEP_NEXT_ARG.bits | Self::VALUE_SEP_NO_SPACE.bits;
    }
}

impl ArgOpts {
    /// Whether the options specify the support for `count` hyphens.
    ///
    /// If the options don't specify any support for hyphens assume all are supported.
    pub const fn is_hyphen_count(self, count: usize) -> bool {
        (count == 1 && self.contains(Self::SINGLE_HYPHEN))
            || (count == 2 && self.contains(Self::DOUBLE_HYPHEN))
            || ((count == 1 || count == 2) && !self.intersects(Self::ALL_HYPHEN))
    }

    /// Whether the option value is optional.
    pub const fn is_value_optional(self) -> bool {
        self.contains(Self::VALUE_OPTIONAL)
    }

    /// Whether the beginning of `s` match any of the value seperator options specified.
    ///
    /// If one seperator option matches `out_sep_len` will be set to the char-length of
    /// the seperator or [`None`] if the value is supposed to be in the next argument.
    ///
    /// If no seperator options are set assumes all except [`ArgOpts::VALUE_SEP_NO_SPACE`].
    pub(super) fn matches_value_sep(mut self, s: &str, out_sep_len: &mut Option<usize>) -> bool {
        if !self.intersects(ArgOpts::ALL_VALUE_SEP) {
            self |= ArgOpts::ALL_VALUE_SEP.difference(ArgOpts::VALUE_SEP_NO_SPACE);
        }

        let c = s.chars().next();
        let (result, sep_len) = match c {
            Some('=') if self.contains(Self::VALUE_SEP_EQUALS) => (true, Some(1)),
            None if self.contains(Self::VALUE_SEP_NEXT_ARG) => (true, None),
            Some(_) if self.contains(Self::VALUE_SEP_NO_SPACE) => (true, Some(0)),
            None if self.contains(Self::VALUE_OPTIONAL) => (true, Some(0)),
            _ => (false, None),
        };
        *out_sep_len = sep_len;
        result
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[must_use]
pub struct ArgDef<'s, 'a> {
    pub arg: Arg,
    pub name: &'s str,
    pub alias: &'a [(&'a str, Option<ArgOpts>)],
    pub opts: ArgOpts,
}

impl<'s, 'a> ArgDef<'s, 'a> {
    /// Set the `alias`(s) for this definition, each alias can have their own [`ArgOpts`]
    /// which override the default [`opts`](ArgDef::opts) when set.
    pub const fn with_alias<'b>(self, alias: &'b [(&'b str, Option<ArgOpts>)]) -> ArgDef<'s, 'b> {
        ArgDef {
            alias,
            arg: self.arg,
            name: self.name,
            opts: self.opts,
        }
    }

    /// Set the options for this definition.
    pub const fn with_opts(mut self, opts: ArgOpts) -> ArgDef<'s, 'a> {
        self.opts = opts;
        self
    }

    /// Set as an argument requiring two `-`.
    pub const fn long(mut self) -> ArgDef<'s, 'a> {
        self.opts = self.opts.union(ArgOpts::DOUBLE_HYPHEN);
        self
    }

    /// Set as an argument requiring one `-`.
    pub const fn short(mut self) -> ArgDef<'s, 'a> {
        self.opts = self.opts.union(ArgOpts::SINGLE_HYPHEN);
        self
    }

    /// Iterate over the default and all aliases of this arg def.
    pub const fn iter(&self) -> ArgDefIter<'_> {
        ArgDefIter {
            alias_index: None,
            arg_def: self,
        }
    }

    /// Generate individual arguments from this argument definition and a `value`.
    ///
    /// The `value` is ignored if this definition is a [`Arg::Flag`].
    pub fn format(&self, value: Option<&str>) -> impl Iterator<Item = String> + Display {
        let ArgDef {
            arg, name, opts, ..
        } = *self;

        match arg {
            Arg::Flag if opts.is_empty() => {
                let second_hyphen = if self.name.len() > 1 { "-" } else { "" };

                FormattedArg::One(format!("-{}{}", second_hyphen, self.name))
            }
            Arg::Flag => {
                let second_hyphen = if opts.contains(ArgOpts::SINGLE_HYPHEN) {
                    ""
                } else {
                    "-"
                };

                FormattedArg::One(format!("-{}{}", second_hyphen, self.name))
            }
            Arg::Option => {
                assert!(value.is_some() || (value.is_none() && opts.is_value_optional()));

                let sep = if value.is_none() && opts.is_value_optional() {
                    None
                } else if opts.contains(ArgOpts::VALUE_SEP_EQUALS) {
                    Some("=")
                } else if opts.contains(ArgOpts::VALUE_SEP_NO_SPACE) {
                    Some("")
                } else {
                    None
                };

                let second_hyphen = if opts.contains(ArgOpts::SINGLE_HYPHEN) {
                    ""
                } else if opts.contains(ArgOpts::DOUBLE_HYPHEN) || name.len() > 1 {
                    "-"
                } else {
                    ""
                };

                if let Some(sep) = sep {
                    let f = format!("-{}{}{}{}", second_hyphen, name, sep, value.unwrap());
                    FormattedArg::One(f)
                } else {
                    let f = format!("-{}{}", second_hyphen, name);
                    if let Some(value) = value {
                        FormattedArg::Two(f, value.into())
                    } else {
                        FormattedArg::One(f)
                    }
                }
            }
        }
    }
}

/// An iterator that iterates over the default and all aliases of an [`ArgDef`].
pub struct ArgDefIter<'d> {
    arg_def: &'d ArgDef<'d, 'd>,
    alias_index: Option<usize>,
}

impl<'d> Iterator for ArgDefIter<'d> {
    type Item = (&'d str, ArgOpts);

    fn next(&mut self) -> Option<Self::Item> {
        let ArgDefIter {
            arg_def,
            alias_index,
        } = self;

        if let Some(i) = alias_index {
            if *i >= arg_def.alias.len() {
                None
            } else {
                let (name, opts) = arg_def.alias[*i];
                *i += 1;

                Some((name, opts.unwrap_or(arg_def.opts)))
            }
        } else {
            *alias_index = Some(0);
            Some((arg_def.name, arg_def.opts))
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum FormattedArg {
    None,
    One(String),
    Two(String, String),
}

impl Iterator for FormattedArg {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Two(first, second) => {
                let first = std::mem::take(first);
                let second = std::mem::take(second);
                *self = Self::One(second);
                Some(first)
            }
            Self::One(first) => {
                let first = std::mem::take(first);
                *self = Self::None;
                Some(first)
            }
            _ => None,
        }
    }
}

impl Display for FormattedArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Two(first, second) => write!(f, "{} {}", first, second),
            Self::One(first) => write!(f, "{}", first),
            Self::None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_flag() {
        const DEF: ArgDef = Arg::flag("n");
        const DEF_LONG: ArgDef = Arg::flag("name");

        assert_eq!(&DEF.format(None).to_string(), "-n");
        assert_eq!(&DEF.format(Some("hallo")).to_string(), "-n");

        assert_eq!(&DEF_LONG.format(None).to_string(), "--name");
        assert_eq!(&DEF_LONG.format(Some("hallo")).to_string(), "--name");

        let def = Arg::flag("n").with_opts(ArgOpts::DOUBLE_HYPHEN);
        let long_def = Arg::flag("name").with_opts(ArgOpts::SINGLE_HYPHEN);

        assert_eq!(&def.format(None).to_string(), "--n");
        assert_eq!(&def.format(Some("hallo")).to_string(), "--n");
        assert_eq!(&long_def.format(None).to_string(), "-name");
        assert_eq!(&long_def.format(Some("hallo")).to_string(), "-name");
    }

    #[test]
    fn format_option() {
        const DEF: ArgDef = Arg::option("n");
        const DEF_LONG: ArgDef = Arg::option("name");

        assert_eq!(&DEF.format(Some("value")).to_string(), "-n value");
        assert_eq!(&DEF_LONG.format(Some("value")).to_string(), "--name value");

        const DEF1: ArgDef = Arg::option("n").with_opts(ArgOpts::DOUBLE_HYPHEN);
        const DEF1_LONG: ArgDef = Arg::option("name").with_opts(ArgOpts::SINGLE_HYPHEN);

        assert_eq!(&DEF1.format(Some("value")).to_string(), "--n value");
        assert_eq!(&DEF1_LONG.format(Some("value")).to_string(), "-name value");

        let def = Arg::option("n").with_opts(ArgOpts::VALUE_SEP_EQUALS);
        let def_long = Arg::option("name").with_opts(ArgOpts::VALUE_SEP_EQUALS);

        assert_eq!(&def.format(Some("value")).to_string(), "-n=value");
        assert_eq!(&def_long.format(Some("value")).to_string(), "--name=value");

        let def = Arg::option("n").with_opts(ArgOpts::VALUE_SEP_NO_SPACE);
        let def_long = Arg::option("name").with_opts(ArgOpts::VALUE_SEP_NO_SPACE);

        assert_eq!(&def.format(Some("value")).to_string(), "-nvalue");
        assert_eq!(&def_long.format(Some("value")).to_string(), "--namevalue");

        let def = Arg::option("n").with_opts(ArgOpts::VALUE_SEP_NEXT_ARG);
        let def_long = Arg::option("name").with_opts(ArgOpts::VALUE_SEP_NEXT_ARG);

        assert_eq!(&def.format(Some("value")).to_string(), "-n value");
        assert_eq!(&def_long.format(Some("value")).to_string(), "--name value");

        let def =
            Arg::option("name").with_opts(ArgOpts::SINGLE_HYPHEN | ArgOpts::VALUE_SEP_NEXT_ARG);
        let mut iter = def.format(Some("value"));

        assert_eq!(iter.next(), Some(String::from("-name")));
        assert_eq!(iter.next(), Some(String::from("value")));

        let def = Arg::option("name").with_opts(
            ArgOpts::DOUBLE_HYPHEN | ArgOpts::VALUE_SEP_NEXT_ARG | ArgOpts::VALUE_OPTIONAL,
        );
        let mut iter = def.format(None);

        assert_eq!(iter.next(), Some(String::from("--name")));
        assert_eq!(iter.next(), None);
    }
}
