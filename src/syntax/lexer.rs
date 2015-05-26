//! This module defines a lexer to recognize tokens of the shell language.

use super::token::Token;
use super::token::Token::*;
use self::TokenOrLiteral::*;

#[derive(PartialEq, Eq, Debug)]
enum TokenOrLiteral {
    Tok(Token),
    Lit(char),
}

/// Converts raw characters into shell tokens.
pub struct Lexer<I: Iterator<Item = char>> {
    inner: ::std::iter::Peekable<I>,
    peeked: Option<TokenOrLiteral>,
}

impl<I: Iterator<Item = char>> Lexer<I> {
    /// Creates a new Lexer from any char iterator.
    pub fn new(iter: I) -> Lexer<I> {
        Lexer {
            inner: iter.peekable(),
            peeked: None,
        }
    }

    #[inline]
    fn next_is(&mut self, c: char) -> bool {
        let is = self.inner.peek() == Some(&c);
        if is { self.inner.next(); }
        is
    }

    fn concat_matching<P: Fn(char) -> bool>(&mut self, initial: Option<char>, predicate: P) -> String {
        let mut s = String::new();
        if initial.is_some() {
            s.push(initial.unwrap());
        }

        // NB: Can't use filter here because it will advance the iterator too far.
        loop {
            match self.inner.peek() {
                Some(&c) if predicate(c) => s.push(self.inner.next().unwrap()),
                _ => break,
            }
        }

        s
    }

    fn next_internal(&mut self) -> Option<TokenOrLiteral> {
        if self.peeked.is_some() {
            return self.peeked.take();
        }

        let cur = match self.inner.next() {
            Some(c) => c,
            None => return None,
        };

        let tok = match cur {
            '\n' => Newline,
            '{' => CurlyOpen,
            '}' => CurlyClose,
            '!' => Bang,
            '~' => Tilde,
            '"' => DoubleQuote,
            '`' => Backtick,

            ';' => if self.next_is(';') { DSemi } else { Semi },
            '&' => if self.next_is('&') { AndIf } else { Amp  },
            '|' => if self.next_is('|') { OrIf  } else { Pipe },

            '(' => ParenOpen,
            ')' => ParenClose,

            '$' => if self.next_is('@') {
                ParamAt
            } else if self.next_is('*') {
                ParamStar
            } else if self.next_is('#') {
                ParamPound
            } else if self.next_is('?') {
                ParamQuestion
            } else if self.next_is('-') {
                ParamDash
            } else if self.next_is('$') {
                ParamDollar
            } else if self.next_is('!') {
                ParamBang
            } else {
                match self.inner.peek() {
                    // Positional parameters are 0-9, so we only
                    // need to check a single digit ahead.
                    Some(&d) if d.is_digit(10) => {
                        self.inner.next();
                        ParamPositional(d.to_digit(10).unwrap() as u8)
                    },
                    _ => Dollar,
                }
            },

            '<' => if self.next_is('<') {
                if self.next_is('-') { DLessDash } else { DLess }
            } else if self.next_is('&') {
                if self.next_is('-') { LessAndDash } else { LessAnd }
            } else if self.next_is('>') {
                LessGreat
            } else {
                Less
            },

            '>' => if self.next_is('&') {
                if self.next_is('-') { GreatAndDash } else { GreatAnd }
            } else if self.next_is('>') {
                DGreat
            } else if self.next_is('|') {
                Clobber
            } else {
                Great
            },

            '#' => Comment(self.concat_matching(None, |c| c != '\n')),
            '\'' => {
                let quot = self.concat_matching(None, |c| c != '\'');
                self.next_is('\''); // Make sure we consume the closing single quote
                SingleQuoted(quot)
            },

            // Newlines are valid whitespace, however, we want to tokenize them separately!
            c if c.is_whitespace() =>
                Whitespace(self.concat_matching(Some(c), |c| c.is_whitespace() && c != '\n')),
            c => return Some(Lit(c)),
        };

        Some(Tok(tok))
    }
}

impl<I: Iterator<Item = char>> Iterator for Lexer<I> {
    type Item = Token;

    fn next(&mut self) -> Option<Token> {
        fn name_start_char(c: char) -> bool {
            c == '_' || c.is_alphabetic()
        }

        fn name_char(c: char) -> bool {
            c.is_digit(10) || name_start_char(c)
        }

        fn is_name(s: &String) -> bool {
            s.chars().all(name_char)
        }

        match self.next_internal() {
            None => None,
            Some(Tok(t)) => Some(t),
            Some(Lit(c)) => {
                let maybe_name = name_start_char(c);
                let mut word = String::new();
                word.push(c);

                loop {
                    match self.next_internal() {
                        // If we hit a token, delimit the current word w/o losing the token
                        Some(Tok(t)) => {
                            debug_assert_eq!(self.peeked, None);
                            self.peeked = Some(Tok(t));
                            break;
                        },

                        // If we have a name candidate and hit an '=' this is an assignment token,
                        // and we'll let the parser figure out what the assignment value actually is
                        // (since it may be an actual expression).
                        Some(Lit('=')) if maybe_name && is_name(&word) => return Some(Assignment(word)),

                        // Make sure we delimit valid names whenever a non-name char comes along
                        Some(Lit(c)) if maybe_name && !name_char(c) => {
                            debug_assert_eq!(self.peeked, None);
                            self.peeked = Some(Lit(c));
                            return Some(Name(word));
                        },

                        // Otherwise, keep consuming characters for the literal
                        Some(Lit(c)) => word.push(c),
                        None => break,
                    }
                }

                if maybe_name && is_name(&word) {
                    Some(Name(word))
                } else {
                    Some(Literal(word))
                }
            },
        }
    }
}

#[cfg(test)]
mod test {
    use syntax::token::Token;
    use syntax::token::Token::*;

    macro_rules! check_tok {
        ($fn_name:ident, $tok:expr) => {
            #[test]
            #[allow(non_snake_case)]
            fn $fn_name() {
                let s = format!("{}", $tok);
                let mut lex = super::Lexer::new(s.chars());
                assert_eq!($tok, lex.next().unwrap());
            }
        }
    }

    macro_rules! lex_str {
        ($fn_name:ident, $s:expr, $($tok:expr),+ ) => {
            #[test]
            #[allow(non_snake_case)]
            fn $fn_name() {
                let lex = super::Lexer::new($s.chars());
                let tokens: Vec<Token> = lex.collect();
                assert_eq!(tokens, vec!( $($tok),+ ));
            }
        }
    }

    check_tok!(check_Newline, Newline);
    check_tok!(check_ParenOpen, ParenOpen);
    check_tok!(check_ParenClose, ParenClose);
    check_tok!(check_CurlyOpen, CurlyOpen);
    check_tok!(check_CurlyClose, CurlyClose);
    check_tok!(check_Dollar, Dollar);
    check_tok!(check_Bang, Bang);
    check_tok!(check_Semi, Semi);
    check_tok!(check_Amp, Amp);
    check_tok!(check_Less, Less);
    check_tok!(check_Great, Great);
    check_tok!(check_Pipe, Pipe);
    check_tok!(check_Tilde, Tilde);
    check_tok!(check_DoubleQuote, DoubleQuote);
    check_tok!(check_Backtick, Backtick);
    check_tok!(check_AndIf, AndIf);
    check_tok!(check_OrIf, OrIf);
    check_tok!(check_DSemi, DSemi);
    check_tok!(check_DLess, DLess);
    check_tok!(check_DGreat, DGreat);
    check_tok!(check_GreatAnd, GreatAnd);
    check_tok!(check_LessAnd, LessAnd);
    check_tok!(check_GreatAndDash, GreatAndDash);
    check_tok!(check_LessAndDash, LessAndDash);
    check_tok!(check_DLessDash, DLessDash);
    check_tok!(check_Clobber, Clobber);
    check_tok!(check_LessGreat, LessGreat);
    check_tok!(check_ParamAt, ParamAt);
    check_tok!(check_ParamStar, ParamStar);
    check_tok!(check_ParamPound, ParamPound);
    check_tok!(check_ParamQuestion, ParamQuestion);
    check_tok!(check_ParamDash, ParamDash);
    check_tok!(check_ParamDollar, ParamDollar);
    check_tok!(check_ParamBang, ParamBang);
    check_tok!(check_Whitespace, Whitespace(" \t\r".to_string()));
    check_tok!(check_Name, Name("abc_23_defg".to_string()));
    check_tok!(check_Literal, Literal(",abcdefg80hijklmno-p".to_string()));
    check_tok!(check_ParamPositional, ParamPositional(9));
    check_tok!(check_Comment, Comment("This is some comment. Foo bar".to_string()));
    check_tok!(check_SingleQuoted, SingleQuoted("Hello world\nGood bye\n".to_string()));
    check_tok!(check_Assignment, Assignment("foobar".to_string()));

    lex_str!(check_greedy_Amp,    "&&&",  AndIf, Amp);
    lex_str!(check_greedy_Pipe,   "|||",  OrIf, Pipe);
    lex_str!(check_greedy_Semi,   ";;;",  DSemi, Semi);
    lex_str!(check_greedy_Less,   "<<<",  DLess, Less);
    lex_str!(check_greedy_Great,  ">>>",  DGreat, Great);
    lex_str!(check_greedy_Dollar, "$$$",  ParamDollar, Dollar);
    lex_str!(check_greedy_Less2,  "<<<-", DLess, Less, Literal("-".to_string()));

    lex_str!(check_Assignment_and_value, "foobar=test", Assignment("foobar".to_string()), Name("test".to_string()));
    lex_str!(check_bad_Assigmnent_and_value, "5foobar=test", Literal("5foobar=test".to_string()));
    lex_str!(check_Literal_and_Name_combo, "hello ,asdf5_ 6world __name ^?.@abc _test2",
             Name("hello".to_string()),
             Whitespace(" ".to_string()),
             Literal(",asdf5_".to_string()),
             Whitespace(" ".to_string()),
             Literal("6world".to_string()),
             Whitespace(" ".to_string()),
             Name("__name".to_string()),
             Whitespace(" ".to_string()),
             Literal("^?.@abc".to_string()),
             Whitespace(" ".to_string()),
             Name("_test2".to_string())
     );
}