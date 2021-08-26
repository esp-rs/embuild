use super::{Arg, ArgDef};

#[derive(PartialEq, Eq, Debug)]
pub enum ParseError {
    NotFound,
}

impl std::error::Error for ParseError {}
impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

pub type Result<T> = std::result::Result<T, ParseError>;

impl super::ArgDef<'_, '_> {
    /// Parse this argument definition from `args` at offset `i`.
    ///
    /// This will remove the element(s) from `args` at position `i` that correspond to
    /// this argument and return the potential value of this argument (which is also
    /// removed from `args`).
    pub fn parse(&self, i: usize, args: &mut Vec<String>) -> Result<Option<String>> {
        let arg = &args[i];

        let hyphen_count = arg.chars().take_while(|s| *s == '-').count();
        for (arg_name, arg_opts) in self.iter() {
            if !arg_opts.is_hyphen_count(hyphen_count) {
                continue;
            }

            let arg = &arg[hyphen_count..];
            match self.arg {
                Arg::Flag => {
                    if arg_name == arg {
                        args.remove(i);
                        return Ok(None);
                    }
                }
                Arg::Option => {
                    let mut sep_len = None;

                    if !arg.starts_with(arg_name)
                        || !arg_opts.matches_value_sep(&arg[arg_name.len()..], &mut sep_len)
                    {
                        continue;
                    }

                    if let Some(sep_len) = sep_len {
                        let value = arg[arg_name.len() + sep_len..].to_owned();
                        args.remove(i);

                        if arg_opts.is_value_optional() && sep_len == 0 && value.is_empty() {
                            return Ok(None);
                        } else {
                            return Ok(Some(value));
                        }
                    } else if arg_opts.is_value_optional()
                            // check if the next arg starts with a `-`
                            && args[i+1..].first().iter().any(|val| val.starts_with('-'))
                    {
                        args.remove(i);
                        return Ok(None);
                    } else {
                        let end_index = (i + 1).min(args.len() - 1);
                        return Ok(args.drain(i..=end_index).nth(1));
                    }
                }
            }
        }

        Err(ParseError::NotFound)
    }
}

pub trait ParseFrom<const N: usize> {
    type R;

    fn parse_from(&self, args: &mut Vec<String>) -> Self::R;
}

impl<'a, 'b, const N: usize> ParseFrom<N> for [&ArgDef<'a, 'b>; N] {
    type R = [Result<Vec<String>>; N];

    /// Parse all definitions from `args` remove all arguments that match any definition.
    fn parse_from(&self, args: &mut Vec<String>) -> Self::R {
        const INIT: Result<Vec<String>> = Err(ParseError::NotFound);
        let mut results = [INIT; N];

        let mut i = 0;
        while i < args.len() {
            let mut removed = false;
            for (def_i, def) in self.iter().enumerate() {
                let result = def.parse(i, args);
                if let Ok(result) = result {
                    removed = true;

                    if let Ok(ref mut results) = results[def_i] {
                        if let Some(result) = result {
                            results.push(result);
                        }
                    } else {
                        results[def_i] = Ok(result.map(|v| vec![v]).unwrap_or_else(Vec::default));
                    }
                    break;
                }
            }

            if !removed {
                i += 1;
            }
        }

        results
    }
}

impl<'a, 'b> ParseFrom<1> for ArgDef<'a, 'b> {
    type R = Result<Vec<String>>;

    /// Parse this definition from `args` remove all arguments that match this definition.
    fn parse_from(&self, args: &mut Vec<String>) -> Result<Vec<String>> {
        let mut result: Result<Vec<String>> = Err(ParseError::NotFound);

        let mut i = 0;
        while i < args.len() {
            let value = self.parse(i, args);

            if let Ok(value) = value {
                if let Ok(ref mut result) = result {
                    if let Some(value) = value {
                        result.push(value);
                    }
                } else {
                    result = Ok(value.map(|v| vec![v]).unwrap_or_else(Vec::default));
                }
            } else {
                i += 1;
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::super::ArgOpts;
    use super::*;

    #[test]
    fn parse() {
        let mut args = [
            "arg0",
            "arg1",
            "--flag",
            "-f",
            "--f",
            "-flag",
            "-avalue1",
            "-a",
            "value2",
            "arg2",
            "--a",
            "value3",
            "-a=value4",
            "arg3",
        ]
        .iter()
        .map(|&s| s.to_owned())
        .collect::<Vec<_>>();

        let flag_single_hyphen = Arg::flag("flag").with_opts(ArgOpts::SINGLE_HYPHEN);
        let flag_double_hyphen = Arg::flag("flag").with_opts(ArgOpts::DOUBLE_HYPHEN);
        let f = Arg::flag("f");
        let a_no_space = Arg::option("a").with_opts(ArgOpts::VALUE_SEP_NO_SPACE);
        let a_space = Arg::option("a").with_opts(ArgOpts::VALUE_SEP_NEXT_ARG);
        let a_equals = Arg::option("a").with_opts(ArgOpts::VALUE_SEP_EQUALS);

        let [flag_single_hyphen, flag_double_hyphen, f, a_equals, a_no_space, a_space] = [
            &flag_single_hyphen,
            &flag_double_hyphen,
            &f,
            &a_equals,
            &a_no_space,
            &a_space,
        ]
        .parse_from(&mut args);

        assert_eq!(flag_single_hyphen, Ok(vec![]));
        assert_eq!(flag_double_hyphen, Ok(vec![]));
        assert_eq!(f, Ok(vec![]));
        assert_eq!(a_no_space, Ok(vec!["value1".to_owned()]));
        assert_eq!(a_space, Ok(vec!["value2".to_owned(), "value3".to_owned()]));
        assert_eq!(a_equals, Ok(vec!["value4".to_owned()]));

        let mut iter = args.iter().map(String::as_str);
        assert_eq!(iter.next(), Some("arg0"));
        assert_eq!(iter.next(), Some("arg1"));
        assert_eq!(iter.next(), Some("arg2"));
        assert_eq!(iter.next(), Some("arg3"));
        assert_eq!(iter.next(), None);
    }
}
