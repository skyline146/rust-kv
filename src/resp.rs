use std::{io, str::from_utf8};

pub struct Command<'a> {
    pub name: &'a str,
    pub args: Vec<&'a str>,
}

pub fn parse(buf: &[u8]) -> Result<(Vec<Command<'_>>, usize), io::Error> {
    let mut buf = from_utf8(buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if let None = buf.find(|b| b == '\n') {
        // no \n in buffer, skipping
        return Ok((vec![], 0));
    };

    let mut cmds = Vec::new();
    let mut end = 0;
    while buf.len() > 0 {
        let (cmd, next) = if buf.starts_with("*") {
            // return parse_resp(buf);
            unimplemented!("TODO parse_resp");
        } else {
            match parse_inline(buf) {
                Ok(res) => res,
                Err(_) => break,
            }
        };

        if cmd.name != "" {
            cmds.push(cmd);
        }

        end += next;
        buf = &buf[next..];
    }

    Ok((cmds, end))
}

fn parse_inline<'a>(line: &'a str) -> Result<(Command<'a>, usize), io::Error> {
    let mut cmd = Command {
        name: "",
        args: Vec::new(),
    };

    let Some(end) = line.find('\n') else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "no \\n in line",
        ));
    };

    let mut line = line[..end].split_whitespace();
    if let Some(name) = line.next() {
        cmd.name = name;
    };

    for s in line {
        cmd.args.push(s);
    }

    Ok((cmd, end + 1))
}

fn parse_resp(_buf: &str) -> Result<(Command, usize), io::Error> {
    let cmd = Command {
        name: "",
        args: Vec::new(),
    };

    Ok((cmd, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(buf: &[u8]) -> (Vec<(&str, Vec<&str>)>, usize) {
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
        assert_eq!(p(buf), (vec![("PING", vec![])], buf.len()));
    }

    #[test]
    fn single_command_bare_lf() {
        let buf = b"PING\n";
        assert_eq!(p(buf), (vec![("PING", vec![])], buf.len()));
    }

    #[test]
    fn args_and_whitespace() {
        let buf = b"  SET   key\tvalue  \r\n";
        assert_eq!(p(buf), (vec![("SET", vec!["key", "value"])], buf.len()));
    }

    #[test]
    fn utf8_args() {
        let buf = "SET ключ значение\r\n".as_bytes();
        assert_eq!(p(buf), (vec![("SET", vec!["ключ", "значение"])], buf.len()));
    }

    #[test]
    fn cr_inside_line_splits_args() {
        // \r inside command equals whitespace
        let buf = b"SET a\rb\r\n";
        assert_eq!(p(buf), (vec![("SET", vec!["a", "b"])], buf.len()));
    }

    #[test]
    fn pipeline_mixed_line_endings() {
        let buf = b"PING\nECHO hi\r\nGET a\n";
        assert_eq!(
            p(buf),
            (
                vec![("PING", vec![]), ("ECHO", vec!["hi"]), ("GET", vec!["a"])],
                buf.len()
            )
        );
    }

    #[test]
    fn incomplete_tail_is_not_consumed() {
        let complete = b"GET a\r\n";
        let buf = b"GET a\r\nSET b";
        assert_eq!(p(buf), (vec![("GET", vec!["a"])], complete.len()));
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
        assert_eq!(p(buf), (vec![("PING", vec![])], buf.len()));
    }

    #[test]
    fn invalid_utf8_is_error_not_panic() {
        assert!(parse(b"GET \xff\r\n").is_err());
    }

    #[test]
    fn any_split_point() {
        // данные пришли двумя кусками, разрез в любом месте
        let input: &[u8] = b"SET a 1\r\nGET a\n\r\n  \r\nPING\r\n";
        let (all, total) = p(input);
        assert_eq!(total, input.len());

        for i in 0..=input.len() {
            let head = &input[..i];
            let (mut cmds, n1) = p(head);

            // съесть надо ровно до последнего \n в куске: не меньше и не больше
            let expected = head
                .iter()
                .rposition(|&b| b == b'\n')
                .map_or(0, |pos| pos + 1);
            assert_eq!(n1, expected, "разрез на {i}");

            // недоеденный хвост + дошедшие данные
            let (rest, n2) = p(&input[n1..]);
            cmds.extend(rest);
            assert_eq!(cmds, all, "разрез на {i}");
            assert_eq!(n1 + n2, input.len(), "разрез на {i}");
        }
    }

    #[test]
    fn byte_by_byte() {
        // как в event loop: каждый read приносит по одному байту
        let input = b"SET a 1\r\nGET a\n\r\nPING\r\n";
        let mut buf: Vec<u8> = Vec::new();
        let mut got: Vec<String> = Vec::new();

        for &b in input {
            buf.push(b);
            let (cmds, n) = parse(&buf).expect("parse вернул Err");
            for c in cmds {
                // команды ссылаются на buf, поэтому копируем их до drain
                got.push(format!("{} {:?}", c.name, c.args));
            }
            buf.drain(..n);
        }

        assert!(buf.is_empty(), "в буфере остался мусор: {buf:?}");
        assert_eq!(got, ["SET [\"a\", \"1\"]", "GET [\"a\"]", "PING []"]);
    }
}
