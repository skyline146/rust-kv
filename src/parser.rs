use std::{
    cmp::min,
    io,
    str::{Chars, from_utf8},
};

const MAX_ARRAY_LENGTH: u32 = 1024000;
const BULK_STRING_MAX_BYTES: u32 = 536870912;
const MAX_STRING_PREALLOCATE_BYTES: u32 = 16384;

pub struct Command {
    pub name: String,
    pub args: Vec<Vec<u8>>,
}

pub fn parse(buf: &[u8]) -> io::Result<(Vec<Command>, usize)> {
    let mut buf = from_utf8(buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if let None = buf.find(|b| b == '\n') {
        return Ok((vec![], 0)); // no \n in buffer, skipping
    };

    let mut cmds = Vec::new();
    let mut end = 0;
    while buf.len() > 0 {
        let parser = if buf.starts_with("*") {
            parse_resp
        } else {
            parse_inline
        };

        let (cmd, next) = match parser(buf) {
            Ok(res) => res,
            Err(ref e) if e.kind() == io::ErrorKind::InvalidInput => break,
            Err(e) => return Err(e),
        };

        if cmd.name != "" {
            cmds.push(cmd);
        }

        end += next;
        buf = &buf[next..];
    }

    Ok((cmds, end))
}

fn parse_inline(buf: &str) -> io::Result<(Command, usize)> {
    let mut cmd = Command {
        name: String::new(),
        args: Vec::new(),
    };

    let Some(end) = buf.find('\n') else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "no \\n in line",
        ));
    };

    let mut line = buf[..end].split_whitespace();
    if let Some(name) = line.next() {
        cmd.name = name.to_string();
    };

    for s in line {
        cmd.args.push(s.into());
    }

    Ok((cmd, end + 1))
}

fn parse_resp(buf: &str) -> io::Result<(Command, usize)> {
    let mut cmd = Command {
        name: String::new(),
        args: Vec::new(),
    };

    let mut end = 0;
    let mut buf = buf.chars();
    buf.next().unwrap(); // skip *
    end += 1;

    let num_elements = parse_number(&mut buf, &mut end)?;
    if num_elements > MAX_ARRAY_LENGTH {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("array len must be le than {MAX_ARRAY_LENGTH}"),
        ));
    }

    for i in 0..num_elements {
        let Some(dollar_sign) = buf.next() else {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not enough"));
        };
        if dollar_sign != '$' {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "argument len must start with $",
            ));
        }
        end += 1;

        let bulk_str_len = parse_number(&mut buf, &mut end)?;
        if bulk_str_len > BULK_STRING_MAX_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("bulk string must be le than {BULK_STRING_MAX_BYTES} bytes"),
            ));
        }

        let mut arg =
            String::with_capacity(min(bulk_str_len, MAX_STRING_PREALLOCATE_BYTES) as usize);
        for _ in 0..bulk_str_len {
            let Some(c) = buf.next() else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("buffer ended earlier than bulk string len {bulk_str_len}"),
                ));
            };
            end += 1;

            arg.push(c);
        }

        let Some(line_break) = buf.next() else {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not enough"));
        };

        let skip_bytes = check_line_break(line_break, &mut buf)?;
        if skip_bytes == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("bulk string must be le than {BULK_STRING_MAX_BYTES} bytes"),
            ));
        }
        end += skip_bytes;

        if i == 0 {
            cmd.name = arg;
        } else {
            cmd.args.push(arg.into_bytes());
        }
    }

    Ok((cmd, end))
}

fn parse_number(buf: &mut Chars<'_>, end: &mut usize) -> io::Result<u32> {
    let mut num = 0;
    while let Some(c) = buf.next() {
        let skip_bytes = check_line_break(c, buf)?;
        if skip_bytes != 0 {
            *end += skip_bytes;
            break;
        }

        let Some(digit) = c.to_digit(10) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{c} cannot be convert to digit"),
            ));
        };

        num *= 10;
        num += digit;

        *end += 1;
    }

    Ok(num)
}

fn check_line_break(c: char, buf: &mut Chars<'_>) -> io::Result<usize> {
    if c == '\r' {
        let Some(next) = buf.next() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "no \\n in line",
            ));
        };

        if next != '\n' {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "\\n must be after \\r in after number of elements in bulk string",
            ));
        }

        return Ok(2);
    }

    if c == '\n' {
        return Ok(1);
    }

    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(buf: &[u8]) -> (Vec<(String, Vec<Vec<u8>>)>, usize) {
        let (cmds, n) = parse(buf).expect("parse err");
        (cmds.into_iter().map(|c| (c.name, c.args)).collect(), n)
    }

    #[test]
    fn empty_buffer() {
        assert_eq!(p(b""), (vec![], 0));
    }

    #[test]
    fn single_command_crlf() {
        let buf = b"PING\r\n";
        assert_eq!(p(buf), (vec![(String::from("PING"), vec![])], buf.len()));
    }

    #[test]
    fn single_command_bare_lf() {
        let buf = b"PING\n";
        assert_eq!(p(buf), (vec![(String::from("PING"), vec![])], buf.len()));
    }

    #[test]
    fn args_and_whitespace() {
        let buf = b"  SET   key\tvalue  \r\n";
        assert_eq!(
            p(buf),
            (
                vec![(String::from("SET"), vec!["key".into(), "value".into()])],
                buf.len()
            )
        );
    }

    #[test]
    fn utf8_args() {
        let buf = "SET ключ значение\r\n".as_bytes();
        assert_eq!(
            p(buf),
            (
                vec![(String::from("SET"), vec!["ключ".into(), "значение".into()])],
                buf.len()
            )
        );
    }

    #[test]
    fn cr_inside_line_splits_args() {
        let buf = b"SET a\rb\r\n";
        assert_eq!(
            p(buf),
            (
                vec![(String::from("SET"), vec!["a".into(), "b".into()])],
                buf.len()
            )
        );
    }

    #[test]
    fn pipeline_mixed_line_endings() {
        let buf = b"PING\nECHO hi\r\nGET a\n";
        assert_eq!(
            p(buf),
            (
                vec![
                    (String::from("PING"), vec![]),
                    (String::from("ECHO"), vec!["hi".into()]),
                    (String::from("GET"), vec!["a".into()])
                ],
                buf.len()
            )
        );
    }

    #[test]
    fn incomplete_tail_is_not_consumed() {
        let complete = b"GET a\r\n";
        let buf = b"GET a\r\nSET b";
        assert_eq!(
            p(buf),
            (
                vec![(String::from("GET"), vec!["a".into()])],
                complete.len()
            )
        );
    }

    #[test]
    fn no_lf_means_incomplete() {
        assert_eq!(p(b"PING"), (vec![], 0));
        assert_eq!(p(b"PING\r"), (vec![], 0));
        assert_eq!(p(b"   "), (vec![], 0));
    }

    #[test]
    fn empty_lines_are_consumed() {
        let buf = b"\r\n\n   \r\n";
        assert_eq!(p(buf), (vec![], buf.len()));

        let buf = b"\r\n\nPING\r\n";
        assert_eq!(p(buf), (vec![(String::from("PING"), vec![])], buf.len()));
    }

    #[test]
    fn invalid_utf8_is_error_not_panic() {
        assert!(parse(b"GET \xff\r\n").is_err());
    }

    #[test]
    fn any_split_point() {
        let input: &[u8] = b"SET a 1\r\nGET a\n\r\n  \r\nPING\r\n";
        let (all, total) = p(input);
        assert_eq!(total, input.len());

        for i in 0..=input.len() {
            let head = &input[..i];
            let (mut cmds, n1) = p(head);

            let expected = head
                .iter()
                .rposition(|&b| b == b'\n')
                .map_or(0, |pos| pos + 1);
            assert_eq!(n1, expected, "разрез на {i}");

            let (rest, n2) = p(&input[n1..]);
            cmds.extend(rest);
            assert_eq!(cmds, all, "разрез на {i}");
            assert_eq!(n1 + n2, input.len(), "разрез на {i}");
        }
    }

    #[test]
    fn byte_by_byte() {
        let input = b"SET a 1\r\nGET a\n\r\nPING\r\n";
        let mut buf: Vec<u8> = Vec::new();
        let mut got: Vec<String> = Vec::new();

        for &b in input {
            buf.push(b);
            let (cmds, n) = parse(&buf).expect("parse вернул Err");
            for c in cmds {
                got.push(format!("{} {:?}", c.name, c.args));
            }
            buf.drain(..n);
        }

        assert!(buf.is_empty(), "в буфере остался мусор: {buf:?}");
        assert_eq!(got, ["SET [[97], [49]]", "GET [[97]]", "PING []"]);
    }

    #[test]
    fn parse_resp_simple() {
        let input = b"*2\r\n$3\r\nGET\r\n$1\r\na\r\n";
        assert_eq!(
            p(input),
            (vec![(String::from("GET"), vec!["a".into()])], input.len())
        );
    }
}
