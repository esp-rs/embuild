/// An iterator that parses a command as windows command-line arguments and returns them
/// as [`String`]s.
///
/// See the MSDN document "Parsing C Command-Line Arguments" at
/// <https://docs.microsoft.com/en-us/cpp/c-language/parsing-c-command-line-arguments>
/// for rules of parsing the windows command line.
pub struct SeperateWindowsArgs<'a> {
    command: &'a str,
    index: usize,
    in_first_argument: bool,
    arg: String,
}

impl<'a> SeperateWindowsArgs<'a> {
    /// Create a new parser from `command`.
    ///
    /// The returned parser does NOT treat the first argument as a program path as
    /// described in the MSDN document.
    pub fn new(command: &'a str) -> SeperateWindowsArgs<'a> {
        SeperateWindowsArgs {
            command,
            index: 0,
            in_first_argument: false,
            arg: String::new(),
        }
    }

    /// Create a new parser from `command` where the first argument is a program path.
    ///
    /// The returned parser does treat the first argument as a program path.
    pub fn new_with_program(command: &'a str) -> SeperateWindowsArgs<'a> {
        SeperateWindowsArgs {
            command,
            index: 0,
            in_first_argument: true,
            arg: String::new(),
        }
    }
}

impl<'a> Iterator for SeperateWindowsArgs<'a> {
    type Item = String;

    /// Parse the command as seperate arguments.
    ///
    /// See the MSDN document "Parsing C Command-Line Arguments" at
    /// <https://docs.microsoft.com/en-us/cpp/c-language/parsing-c-command-line-arguments>
    /// for rules of parsing the windows command line.
    fn next(&mut self) -> Option<Self::Item> {
        let Self {
            ref command,
            index,
            in_first_argument,
            ref mut arg,
        } = *self;

        if index > command.len() {
            return None;
        }

        arg.clear();

        let mut last_char = ' ';
        let mut in_quotes = false;
        let mut consecutive_quotes = 0;
        let mut backslashes = 0_u32;

        /// Append `backslashes` amount of `\` to the argument, or half the amount if
        /// `half` is `true`. Set `backslashes` to zero.
        fn push_backslashes(arg: &mut String, backslashes: &mut u32, half: bool) {
            if *backslashes > 0 {
                let n = (*backslashes >> (half as u32)) as usize;
                arg.reserve(n);
                for _ in 0..n {
                    arg.push('\\');
                }
                *backslashes = 0;
            }
        }

        fn push_quotes(arg: &mut String, quotes: &mut u32, in_quotes: &mut bool) {
            if *quotes > 0 {
                let n = (*quotes >> 1) as usize;
                let is_even = *quotes & 1 == 0;

                arg.reserve(n);
                for _ in 0..n {
                    arg.push('"');
                }
                *quotes = 0;
                *in_quotes = is_even;
            }
        }

        for (index, c) in (&command[index..]).char_indices() {
            match c {
                '"' if in_first_argument => {
                    in_quotes = !in_quotes;
                }
                '"' => {
                    let is_backslash_escaped = backslashes % 2 == 1;
                    push_backslashes(arg, &mut backslashes, true);

                    if is_backslash_escaped {
                        arg.push(c);
                    } else if !in_quotes {
                        in_quotes = true;
                    } else {
                        consecutive_quotes += 1;
                    }
                }
                '\\' => {
                    backslashes += 1;
                    push_quotes(arg, &mut consecutive_quotes, &mut in_quotes);
                }
                // This filters empty arguments which doesn't really conform to the spec
                // but we don't need them.
                ' ' | '\t' if !in_quotes && matches!(last_char, ' ' | '\t') => {}
                ' ' | '\t' => {
                    push_backslashes(arg, &mut backslashes, false);
                    push_quotes(arg, &mut consecutive_quotes, &mut in_quotes);

                    if in_quotes {
                        arg.push(c);
                    } else {
                        self.index += index + 1;
                        self.in_first_argument = false;

                        let mut result = String::with_capacity(arg.len());
                        result.clone_from(arg);
                        return Some(result);
                    }
                }
                c => {
                    push_backslashes(arg, &mut backslashes, false);
                    push_quotes(arg, &mut consecutive_quotes, &mut in_quotes);
                    arg.push(c);
                }
            }
            last_char = c;
        }
        push_backslashes(arg, &mut backslashes, false);
        push_quotes(arg, &mut consecutive_quotes, &mut in_quotes);

        self.index = command.len() + 1;
        if arg.is_empty() {
            None
        } else {
            let mut result = std::mem::replace(arg, String::new());
            result.shrink_to_fit();

            Some(result)
        }
    }
}

/// An iterator that parses a command as unix command-line arguments and returns them
/// as [`String`]s.
///
/// Ported from cmake's [`kwsysSystem__ParseUnixCommand`][1].
///
/// [1]: https://github.com/Kitware/CMake/blob/859241d2bbaae83f06c44bc21ab0dbd38725a3fc/Source/kwsys/System.c#L94
pub struct SeperateUnixArgs<'a> {
    command: &'a str,
    index: usize,
    arg: String,
}

impl<'a> SeperateUnixArgs<'a> {
    /// Create a new parser from `command`.
    pub fn new(command: &'a str) -> SeperateUnixArgs<'a> {
        SeperateUnixArgs {
            command,
            index: 0,
            arg: String::new(),
        }
    }
}

impl<'a> Iterator for SeperateUnixArgs<'a> {
    type Item = String;

    /// Parse the command as seperate arguments.
    ///
    /// Ported from cmake's [`kwsysSystem__ParseUnixCommand`][1].
    ///
    /// [1]: https://github.com/Kitware/CMake/blob/859241d2bbaae83f06c44bc21ab0dbd38725a3fc/Source/kwsys/System.c#L94
    fn next(&mut self) -> Option<Self::Item> {
        let Self {
            ref command,
            index,
            ref mut arg,
        } = *self;

        if index > command.len() {
            return None;
        }

        arg.clear();

        let mut in_argument = false;
        let mut in_single = false;
        let mut in_double = false;
        let mut in_escape = false;

        for (index, c) in (&command[index..]).char_indices() {
            match c {
                c if in_escape => {
                    in_argument = true;
                    arg.push(c);
                    in_escape = false;
                }
                '\\' => {
                    in_escape = true;
                }
                '\'' if !in_double => {
                    in_argument = true;
                    in_single = !in_single;
                }
                '"' if !in_single => {
                    in_argument = true;
                    in_double = !in_double;
                }
                c if !in_argument && c.is_whitespace() => {}
                c if c.is_whitespace() => {
                    if in_single || in_double {
                        arg.push(c);
                    } else {
                        self.index += index + c.len_utf8();

                        let mut result = String::with_capacity(arg.len());
                        result.clone_from(arg);
                        return Some(result);
                    }
                }
                c => {
                    in_argument = true;
                    arg.push(c);
                }
            }
        }

        self.index = command.len() + 1;
        if in_argument {
            let mut result = std::mem::replace(arg, String::new());
            result.shrink_to_fit();

            Some(result)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn seperate_unix_args() {
        let cmd = r#"arg -arg  "hello" "\" $'^`?@¦|''"  '""' ""''\'"" end"#;

        let args = SeperateUnixArgs::new(cmd).collect::<Vec<_>>();
        let mut iter = args.iter().map(|s| &s[..]);

        assert_eq!(iter.next(), Some("arg"));
        assert_eq!(iter.next(), Some("-arg"));
        assert_eq!(iter.next(), Some("hello"));
        assert_eq!(iter.next(), Some(r#"" $'^`?@¦|''"#));
        assert_eq!(iter.next(), Some("\"\""));
        assert_eq!(iter.next(), Some("\'"));
        assert_eq!(iter.next(), Some("end"));
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn seperate_windows_args() {
        let cmd = r#"C:\path\\\" a  a "/\\//^.. "arg with whitespace" 'abc' '"" "'" "''" ""'""" s  " """"   \\\\"" \\\" \\\\\" \\\abc "rest a b   "#;

        let args = SeperateWindowsArgs::new_with_program(cmd).collect::<Vec<_>>();
        let mut iter = args.iter().map(|s| &s[..]);

        assert_eq!(iter.next(), Some(r"C:\path\\\ a  a /\\//^.."));
        assert_eq!(iter.next(), Some("arg with whitespace"));
        assert_eq!(iter.next(), Some("'abc'"));
        assert_eq!(iter.next(), Some("'"));
        assert_eq!(iter.next(), Some("'"));
        assert_eq!(iter.next(), Some("''"));
        assert_eq!(iter.next(), Some("'\" s  "));
        assert_eq!(iter.next(), Some("\""));
        assert_eq!(iter.next(), Some(r"\\"));
        assert_eq!(iter.next(), Some("\\\""));
        assert_eq!(iter.next(), Some(r#"\\""#));
        assert_eq!(iter.next(), Some(r#"\\\abc"#));
        assert_eq!(iter.next(), Some("rest a b   "));
        assert_eq!(iter.next(), None);
    }
}
