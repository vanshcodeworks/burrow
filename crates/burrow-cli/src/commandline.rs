//! Windows command-line splitting for `run = "..."` values.
//!
//! The quoting rules match `CommandLineToArgvW`, so a command written the way a
//! user would type it in `cmd.exe` is split the way the child process expects.

/// Splits a command line into a program and its arguments.
pub fn split(command: &str) -> anyhow::Result<(String, Vec<String>)> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut quoted = false;
    let mut backslashes = 0usize;

    for character in command.chars() {
        match character {
            '\\' => {
                backslashes += 1;
                started = true;
            }
            '"' => {
                current.extend(std::iter::repeat_n('\\', backslashes / 2));
                if backslashes % 2 == 1 {
                    current.push('"');
                } else {
                    quoted = !quoted;
                }
                backslashes = 0;
                started = true;
            }
            value if value.is_whitespace() && !quoted => {
                current.extend(std::iter::repeat_n('\\', backslashes));
                backslashes = 0;
                if started {
                    arguments.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            value => {
                current.extend(std::iter::repeat_n('\\', backslashes));
                backslashes = 0;
                current.push(value);
                started = true;
            }
        }
    }

    if quoted {
        anyhow::bail!("unterminated quote in command `{command}`");
    }
    current.extend(std::iter::repeat_n('\\', backslashes));
    if started {
        arguments.push(current);
    }

    if arguments.is_empty() {
        anyhow::bail!("command is empty");
    }
    let program = arguments.remove(0);
    Ok((program, arguments))
}

#[cfg(test)]
mod tests {
    use super::split;

    #[test]
    fn splits_plain_commands() {
        let (program, args) = split("npm run dev").expect("split");
        assert_eq!(program, "npm");
        assert_eq!(args, ["run", "dev"]);
    }

    #[test]
    fn keeps_quoted_arguments_together() {
        let (program, args) = split(r#""C:\Program Files\node\node.exe" "a b" c"#).expect("split");
        assert_eq!(program, r"C:\Program Files\node\node.exe");
        assert_eq!(args, ["a b", "c"]);
    }

    #[test]
    fn handles_escaped_quotes_and_trailing_backslashes() {
        let (_, args) =
            split(r#"app "say \"hi\"" C:\path\ "C:\path with space\\""#).expect("split");
        assert_eq!(args[0], "say \"hi\"");
        assert_eq!(args[1], r"C:\path\");
        assert_eq!(args[2], r"C:\path with space\");
    }

    #[test]
    fn empty_quoted_arguments_are_preserved() {
        let (_, args) = split(r#"app "" x"#).expect("split");
        assert_eq!(args, ["", "x"]);
    }

    #[test]
    fn rejects_unterminated_quotes() {
        assert!(split(r#"app "unterminated"#).is_err());
        assert!(split("   ").is_err());
    }
}
