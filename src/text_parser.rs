use std::{
    cell::Cell,
    ops::{Bound, RangeBounds},
    str::FromStr,
};

use log::trace;

use crate::{error, error::ParseError, selection::Selection, util};

thread_local!(static LABEL: Cell<&'static str> = Cell::new(""));

#[inline]
pub fn cursor(s: &str) -> Cursor {
    crate::text_parser::Cursor::from(s)
}

fn cursorify<'a, T>(
    mut f: impl FnMut(&'a str) -> Result<(&'a str, T), ParseError>,
) -> impl FnMut(Cursor<'a>) -> Result<(Cursor<'a>, T), ParseError> {
    move |c: Cursor<'a>| (f)(c.str()?).map(|(s, t)| (cursor(s), t))
}

pub trait Parser<'a, T> {
    fn parse(&mut self, s: Cursor<'a>) -> Result<(Cursor<'a>, T), ParseError>;
}

impl<'p, T> Parser<'p, T> for for<'b> fn(&'b str) -> Result<(&'b str, T), ParseError> {
    fn parse(&mut self, c: Cursor<'p>) -> Result<(Cursor<'p>, T), ParseError> {
        trace!("#### fn(&'b str): {s}", s = c.cur.unwrap_or("-"));
        let (s, t) = (self)(c.str()?)?;
        Ok((cursor(s), t))
    }
}

impl<'a, T, F> Parser<'a, T> for F
where
    F: FnMut(Cursor<'a>) -> Result<(Cursor<'a>, T), ParseError>,
{
    fn parse(&mut self, s: Cursor<'a>) -> Result<(Cursor<'a>, T), ParseError> {
        trace!("#### FnMut(SelectableStr): {s}", s = s.cur.unwrap_or("-"));
        (self)(s)
    }
}

impl<'a, T, X> Parser<'a, T>
    for (
        &'a X,
        for<'b> fn(context: &'b X, &'b str) -> Result<(&'b str, T), ParseError>,
    )
{
    fn parse(&mut self, c: Cursor<'a>) -> Result<(Cursor<'a>, T), ParseError> {
        trace!("#### fn(context, &str): {s}", s = c.cur.unwrap_or("-"));
        let (s, t) = (self.1)(self.0, c.str()?)?;
        Ok((cursor(s), t))
    }
}

// pub trait ParserArg<'a> {
//     type ConvertFrom;
//     fn from_cursor(c: Self::ConvertFrom) -> Self;
//     fn to_result_arg(&self) -> Result<&'a str, BadMatch>;
// }

// impl<'a> ParserArg<'a> for &'a str {
//     // fn from_cursor<C: Cursor<'a>>(c: C) -> Self {
//     //     c.as_str()
//     // }
//     type ConvertFrom;
//     fn from_cursor(c: Self::ConvertFrom) -> Self;

//     fn to_result_arg(&self) -> Result<&'a str, BadMatch> {
//         Ok(self)
//     }
// }

// impl<'a> ParserArg<'a> for SelectableStr<'a> {
//     // type Cur = Self;

//     // fn from_cursor<C: Cursor<'a>>(c: C) -> Self {
//     //     self
//     // }
//     type ConvertFrom;
//     fn from_cursor(c: Self::ConvertFrom) -> Self;

//     fn to_result_arg(&self) -> Result<&'a str, BadMatch> {
//         Ok(self.str()?)
//     }
// }

#[derive(Debug, Clone)]
pub struct Cursor<'a> {
    selection: Selection<'a>,
    cur: Option<&'a str>,
    err: Option<ParseError>,
    context: &'a str,
}

// equal and error free
impl<'a> PartialEq for Cursor<'a> {
    #[allow(clippy::match_like_matches_macro)]
    fn eq(&self, other: &Self) -> bool {
        self.selection == other.selection
            && self.cur == other.cur
            && self.context == other.context
            && match (&self.err, &other.err) {
                (None, None) => true,
                _ => false,
            }
    }
}

impl<'a> From<&'a str> for Cursor<'a> {
    #[inline]
    fn from(s: &'a str) -> Self {
        Self {
            selection: Selection::Defaulted(s),
            cur: Some(s),
            err: None,
            context: "",
        }
    }
}

pub trait Bind<T> {
    type Output;
    fn bind(self, t: &mut T) -> Self::Output;
}

// needs macro expansion for i32/f64 etc and for vec extend
impl<'a, C, T> Bind<T> for Result<(C, T), ParseError>
where
    C: Matchable<'a>,
{
    type Output = C;

    fn bind(self, target: &mut T) -> Self::Output {
        let res = self.map(|(s, t)| {
            *target = t;
            s
        });
        match res {
            Ok(s) => s,
            Err(_e) => todo!(),
        }
    }
}

fn start_end<R: RangeBounds<i32>>(rb: R) -> (Option<i32>, Option<i32>) {
    let start = match rb.start_bound() {
        Bound::Included(&i) => Some(i),
        Bound::Excluded(&i) => Some(i + 1),
        Bound::Unbounded => None,
    };
    let end = match rb.end_bound() {
        Bound::Included(&i) => Some(i),
        Bound::Excluded(&i) => Some(i - 1),
        Bound::Unbounded => None,
    };
    (start, end)
}

#[inline]
fn find_first<'a, R, C, F>(cur: C, rb: R, pred: F, action: &'static str, args: &str) -> C
where
    R: RangeBounds<i32>,
    C: Matchable<'a>,
    F: FnMut(char) -> bool,
{
    let Ok(s) = cur.str() else {
        trace!(
            "{label:<20} skipping {action:<10}({args:<10}) = '{inp}'",
            label = LABEL.with(|f| f.get()),
            inp = util::formatter_str(cur.str().unwrap_or_default()),
        );
        return cur;
    };
    let (start, end) = start_end(rb);
    if let Some(end) = end {
        if end < 0 {
            trace!(
                "{label:<20} end<0 {action:<10}({args:<10}) = '{inp}'",
                label = LABEL.with(|f| f.get()),
                inp = util::formatter_str(cur.str().unwrap_or_default()),
            );
            return cur.set_error(ParseError::NoMatch { action, args: "" });
        }
    }
    //  set start to 0, if < 0
    let start = start.unwrap_or_default() as usize;
    let end = end.unwrap_or(i32::MAX) as usize;
    // trace!(">>>> {action} {} -> {}", start, end);

    if let Some((i, _t)) = s.match_indices(pred).nth(0) {
        // trace!(">>>> {action} matched on i={i} t={t} from s={s} s = {start} e = {end}");

        if i >= start && i <= end+1 {
            trace!(
                "{label:<20} {action:<10}('{inp}') => '{out}'",
                label = LABEL.with(|f| f.get()),
                inp = util::formatter_str(cur.str().unwrap_or_default()),
                out = util::formatter_str(&s[i..])
            );
            return cur.set_str(&s[i..]);
        }
    } else {
        let len = s.chars().count();
        if  len >= start && len <= end {
            trace!(
                "{label:<20} {action:<10}('{inp}') => exhausted",
                label = LABEL.with(|f| f.get()),
                inp = util::formatter_str(cur.str().unwrap_or_default()),
                // out = util::formatter_str("")
            );
            return cur.set_str("");

        }
    }
    // not found
    trace!(
        "{label:<20} {action:<10}('{inp}') => None",
        label = LABEL.with(|f| f.get()),
        inp = util::formatter_str(cur.str().unwrap_or_default())
    );
    return cur.set_error(error::failure(action, ""));
}

#[inline]
fn apply<'a, C, F>(cur: C, f: F, msg: &'static str, args: &str) -> C
where
    C: Matchable<'a>,
    F: FnOnce(&str) -> Option<&str>,
{
    match cur.str() {
        Ok(s) => match f(s) {
            Some(s) => {
                trace!(
                    "{label:<20} {msg:<10}({args:<10}) = '{inp}' => '{out}'",
                    label = LABEL.with(|f| f.get()),
                    inp = util::formatter_str(cur.str().unwrap_or_default()),
                    out = util::formatter_str(s)
                );
                cur.set_str(s)
            }
            None => {
                trace!(
                    "{label:<20} {msg:<10}({args:<10}) = '{inp}' => None",
                    label = LABEL.with(|f| f.get()),
                    inp = util::formatter_str(cur.str().unwrap_or_default())
                );
                cur.set_error(error::failure(msg, s))
            }
        },
        _ => {
            trace!(
                "{label:<20} skipping {msg:<10}({args:<10}) = '{inp}'",
                label = LABEL.with(|f| f.get()),
                inp = util::formatter_str(cur.str().unwrap_or_default()),
            );

            cur
        }
    }
}

pub trait Selectable<'a>: Matchable<'a> {
    // fn parse(self) -> std::result::Result<Self::Cursor, BadMatch> {
    //     CursorHelper::parse(self)
    // }
    fn get_selection(&self) -> Result<&'a str, ParseError>;
    fn selection_end(self) -> Self;
    fn selection_start(self) -> Self;

    fn de_nest_tuple<S, T, U>(((s, t), u): ((S, T), U)) -> (S, T, U) {
        (s, t, u)
    }

    // fn parse_selection_to_i32(self, target: &mut i32) -> Result<Self, ParseError> {
    //     let t = self.get_selection()?;
    //     let t = t.parse().map_err(|_e| error::failure("parse i32", t))?;
    //     *target = t;
    //     Ok(self)
    // }

    // fn parse_selection_to_f64(self, target: &mut f64) -> Result<Self, ParseError> {
    //     let t = self.get_selection()?;
    //     let t = t.parse().map_err(|_e| error::failure("parse f64", t))?;
    //     *target = t;
    //     Ok(self)
    // }

    // fn parse_selection_to_str(self, target: &mut &'a str) -> Result<Self, ParseError> {
    //     let t = self.str()?;
    //     *target = t;
    //     Ok(self)
    // }

    // fn parse_selection_to_string(self, target: &mut String) -> Result<Self, ParseError> {
    //     let t = self.str()?.to_string();
    //     *target = t;
    //     Ok(self)
    // }

    // fn parse_selection_as_i32(self) -> Result<(Self::Cursor, i32), BadMatch> {
    //     let (text, me) = self.get_selection()?;
    //     let cur = me.as_cursor();
    //     trace!(
    //         "parse_selection_as_i32({text}) Cursor => '{}'",
    //         formatter(&cur)
    //     );
    //     let i = text
    //         .parse::<i32>()
    //         .map_err(|_e| failure("parse i32", text.len()))?;
    //     let res = (cur, i);
    //     Ok(res)
    // }

    fn parse_selection<T: FromStr>(self) -> Result<Self::TupleReturn<T>, ParseError> {
        let text = self.get_selection()?;
        let cur = self.str()?;
        trace!(
            "parse_selection (FromStr)({text}) Cursor => '{}'",
            util::formatter_str(cur)
        );
        let i = text
            .parse::<T>()
            .map_err(|_e| error::failure("parse", text))?;
        Ok(Self::maybe_detuple((self, i)))
    }

    // fn parse_selection_as_f64(self) -> Result<Self::TupleReturn<f64>, ParseError> {
    //     let text = self.get_selection()?;
    //     let cur = self.str()?;
    //     trace!(
    //         "parse_selection_as_f64({text}) Cursor => '{}'",
    //         util::formatter_str(cur)
    //     );
    //     let i = text
    //         .parse::<f64>()
    //         .map_err(|_e| error::failure("parse f64", text))?;
    //     Ok(Self::maybe_detuple((self, i)))
    // }

    // fn parse_selection_as_i32(self) -> Result<Self::TupleReturn, BadMatch>;

    // fn parse_selection_as_i32(self) -> Result<Self::TupleReturn<i32>, ParseError> {
    //     let text = self.get_selection()?;
    //     let cur = self.str()?;
    //     trace!(
    //         "parse_selection_as_i32({text}) Cursor => '{}'",
    //         util::formatter_str(cur)
    //     );
    //     let i = text
    //         .parse::<i32>()
    //         .map_err(|_e| error::failure("parse i32", text))?;
    //     Ok(Self::maybe_detuple((self, i)))
    // }
    // fn parse_selection_as_str(self) -> Result<(Self, &'a str), BadMatch> {
    //     todo!()
    // }

    fn append_last<X, T>(self, vec: &mut X) -> Self
    where
        // from iter not used but distinguishes the case of Extend by ref
        X: Extend<T> + FromIterator<T>,
        T: FromStr,
    {
        if let Ok(text) = self.get_selection() {
            let res_t = T::from_str(text);
            if let Ok(t) = res_t {
                vec.extend(std::iter::once(t));
            } else {
                return self.set_error(ParseError::NoMatch {
                    action: "",
                    args: "",
                });
            }
        }
        self
    }

    fn select<P>(self, mut parser: P) -> Self
    where
        P: FnMut(Self) -> Self,
    {
        let msg = "select_with";
        let args = "";
        if let Ok(s) = self.str() {
            let t = parser(self.selection_start());
            match t.str() {
                Ok(tt) => {
                    trace!(
                        "{label:<20} {msg:<10}({args:<10}) = '{inp}' => '{out}'",
                        label = LABEL.with(|f| f.get()),
                        inp = util::formatter_str(s),
                        out = util::formatter_str(tt)
                    );
                    return t.set_str(tt).selection_end();
                }
                _ => {
                    trace!(
                        "{label:<20} {msg:<10}({args:<10}) = '{inp}' => None",
                        label = LABEL.with(|f| f.get()),
                        inp = util::formatter_str(s)
                    );
                    return t.set_error(error::failure(msg, s));
                }
            };
        }
        self
    }

    // fn take_last<M, T>(self, mut target: M) -> Self
    // where
    //     M: AsMut<T>,
    //     T: FromStr,
    // {
    //     if let Ok(text) = self.get_selection() {
    //         let res_t = T::from_str(text);
    //         if let Ok(t) = res_t {
    //             *target.as_mut() = t;
    //         } else {
    //             return self.set_error(ParseError::NoMatch {
    //                 action: "take_last",
    //                 args: "",
    //             });
    //         }
    //     }
    //     self
    // }

    fn parse_selection_as_str(self) -> Result<(Self, &'a str), ParseError> {
        let text = self.get_selection()?;
        let res = (self.str()?, text);
        trace!(
            "parse_selection => '{text}' Cursor => '{}'",
            util::formatter(&Some(res.0))
        );
        Ok((self, text))
    }

    // fn parse_selection_to(self) -> Clipboard<'a, Self> {
    //     Clipboard::new(self)
    // }
}

pub trait Matchable<'a>: Sized {
    // type Cursor: Cursor<'a>;
    // type Raw;
    // type CursorWithSelection: Cursor<'a>;

    type TupleReturn<T>;

    fn maybe_detuple<T>(tup: (Self, T)) -> Self::TupleReturn<T>;

    fn str(&self) -> std::result::Result<&'a str, ParseError>;
    fn set_str(self, s: &'a str) -> Self;
    fn set_error(self, e: ParseError) -> Self;

    #[inline]
    fn debug_context(self, span_name: &'static str) -> Self {
        trace!(
            "setting debug_context to {label}",
            label = LABEL.with(|f| {
                f.set(span_name);
                span_name
            })
        );

        self
    }

    fn validate(self) -> std::result::Result<Self, ParseError>;

    fn noop(self) -> Self {
        apply(self, |s| Some(s), "noop", "")
    }

    #[inline]
    fn ws(self) -> Self {
        apply(self, |s| Some(s.trim_start()), "ws", "")
    }

    fn non_ws(self) -> Self {
        apply(
            self,
            |s| Some(s.trim_start_matches(|c: char| !c.is_whitespace())),
            "non_ws",
            "",
        )
    }

    fn hws(self) -> Self {
        apply(
            self,
            |s| Some(s.trim_start_matches(|c: char| c.is_whitespace() && c != '\n' && c != '\r')),
            "hws",
            "",
        )
    }

    // "" means always match. use eos() to test for end of string/strea,
    fn text(self, word: &str) -> Self {
        apply(self, |s| s.strip_prefix(word), "text", word)
    }

    /// text_many(0..1, "word")
    fn maybe(self, word: &str) -> Self {
        apply(self, |s| s.strip_prefix(word).or(Some(s)), "maybe", word)
    }

    fn text_alt(self, words: &[&str]) -> Self {
        apply(
            self,
            |s| {
                for w in words {
                    if s.starts_with(w) {
                        return s.strip_prefix(w);
                    }
                }
                None
            },
            "text_alt",
            words.first().unwrap_or(&"no words"),
        )
    }

    #[allow(clippy::wrong_self_convention)]
    fn text_eos(self) -> Self {
        apply(
            self,
            |s| if s.is_empty() { Some(s) } else { None },
            "eos",
            "",
        )
    }

    #[allow(clippy::wrong_self_convention)]
    fn text_eol(self) -> Self {
        #[allow(clippy::unnecessary_lazy_evaluations)]
        apply(
            self,
            |s| {
                s.is_empty()
                    .then(|| s)
                    .or_else(|| s.strip_prefix("\r\n"))
                    .or_else(|| s.strip_prefix('\n'))
            },
            "eol",
            "",
        )
    }

    // like rusts, skips to beginning of match:  find(find(find("this"))) === find("this")
    #[inline]
    fn find(self, needle: &str) -> Self {
        apply(self, |s| s.find(needle).map(|i| &s[i..]), "find", needle)
    }

    // from Xpath's substring-after.  scan("blob") === find("blob").text("blob")
    // synonyms: from, read, skim, skip_over, consume, scan
    fn scan_text(self, needle: &str) -> Self {
        apply(
            self,
            |s| s.find(needle).map(|i| &s[i + needle.len()..]),
            "scan",
            "needle",
        )
    }

    // read-to-and-over the end of line (or eos)
    // read_eol, skim_eol, skip_over_eof, scan_eol,
    fn scan_eol(self) -> Self {
        const LEN: usize = ("\n").len();
        apply(
            self,
            |s| s.find('\n').map(|i| &s[i + LEN..]).or(Some("")),
            "scan_eol",
            "",
        )
    }

    fn chars_in<R: RangeBounds<i32>>(self, range: R, chars: &[char]) -> Self {
        // trace!("Chats not in {chars:?}");
        find_first(
            self,
            range,
            |c| !chars.contains(&c),
            // |s| Some(s.trim_start_matches(chars)),
            "chars_in",
            "",
        )
    }

    fn chars_not_in<R: RangeBounds<i32>>(self, range: R, chars: &[char]) -> Self {
        // trace!("Chats not in {chars:?}");
        find_first(
            self,
            range,
            |c| chars.contains(&c),
            // |s| Some(s.trim_start_matches(|c: char| !chars.contains(&c))),
            "chars_not_in",
            "",
        )
    }

    fn chars_match<R: RangeBounds<i32>, F>(self, range: R, mut pred: F) -> Self
    where
        F: FnMut(char) -> bool,
    {
        find_first(
            self,
            range,
            |c| !pred(c),
            // |s| Some(s.trim_start_matches(&mut pred)),
            "chars_match",
            "",
        )
    }

    fn digits<R: RangeBounds<i32>>(self, range: R) -> Self {
        find_first(
            self,
            range,
            |c| !c.is_ascii_digit(),
            // |s| Some(s.trim_start_matches(|c: char| c.is_ascii_digit())),
            "digits_m",
            "",
        )
    }

    /// alphanumeric or digit or hyphen (-)
    fn word(self) -> Self {
        apply(
            self,
            |s| {
                Some(s.trim_start_matches(|c: char| {
                    c.is_alphanumeric() || c.is_ascii_digit() || c == '-'
                }))
            },
            "word",
            "",
        )
    }

    fn alphabetics<R: RangeBounds<i32>>(self, range: R) -> Self {
        find_first(
            self,
            range,
            |c| !c.is_alphabetic(),
            // |s| Some(s.trim_start_matches(|c: char| c.is_alphabetic())),
            "alpha_many",
            "",
        )
    }

    fn alphanumerics<R: RangeBounds<i32>>(self, range: R) -> Self {
        find_first(
            self,
            range,
            |c| !c.is_alphanumeric(),
            // |s| Some(s.trim_start_matches(|c: char| c.is_alphanumeric())),
            "alpha_many",
            "",
        )
    }

    // TODO!
    fn repeat<P, R: RangeBounds<i32>>(self, range: R, mut lexer: P) -> Self
    where
        P: FnMut(Self) -> Self,
        Self: Clone,
    {
        let mut str = self;
        for _i in 0..start_end(range).1.unwrap_or(i32::MAX) {
            match (lexer)(str.clone()).validate() {
                Ok(s) => str = s,
                Err(..) => return str,
            }
        }
        str
    }

    fn parse_struct_vec<P, T>(self, mut parser: P) -> Result<Self::TupleReturn<Vec<T>>, ParseError>
    where
        P: FnMut(Self) -> std::result::Result<(Self, T), ParseError>,
        Self: Clone,
        // C: SelectableCursor<'a>
        // A: IntoIterator<Item = T>
    {
        let mut vec = vec![];
        // let mut str = self.str()?;
        let mut str = self;
        loop {
            match (parser)(str.clone()) {
                Ok((s, t)) => {
                    vec.push(t);
                    str = s;
                }
                Err(ParseError::NoMatch { .. }) => {
                    return Ok(Self::maybe_detuple((str, vec)));
                }

                Err(ParseError::Fatal(e)) => {
                    return Err(ParseError::Fatal(e));
                }
            }
        }
    }

    fn parse_struct_vec_to<P, X, T>(self, mut parser: P, vec: &mut X) -> Result<Self, ParseError>
    where
        P: FnMut(Self) -> std::result::Result<(Self, T), ParseError>,
        X: Extend<T>,
        Self: Clone,
        // A: IntoIterator<Item = T>
    {
        let mut str = self; // .str()?;
        loop {
            match (parser)(str.clone()) {
                Ok((s, t)) => {
                    vec.extend(std::iter::once(t));
                    str = s;
                }
                Err(ParseError::NoMatch { .. }) => {
                    return Ok(str); // self.set_str(str)
                }

                Err(ParseError::Fatal(e)) => {
                    return Err(ParseError::Fatal(e));
                }
            }
        }
    }

    fn parse_struct_str<P, T>(self, mut parser: P) -> Result<Self::TupleReturn<T>, ParseError>
    where
        P: FnMut(&str) -> std::result::Result<(&str, T), ParseError>,
    {
        let s: &str = self.str()?;
        let outcome = (parser)(s)?;
        let (s, t): (&str, T) = outcome;
        let cur = self.set_str(s);
        Ok(Self::maybe_detuple((cur, t)))
    }

    fn parse_struct<C, P, T>(self, mut parser: P) -> Result<Self::TupleReturn<T>, ParseError>
    where
        P: FnMut(Self) -> std::result::Result<(Self, T), ParseError>,
    {
        let outcome = (parser)(self)?;
        let (s, t): (Self, T) = outcome;
        Ok(Self::maybe_detuple((s, t)))
    }

    fn parse_with<P, F, T>(self, mut parser: P, save_func: F) -> Result<Self, ParseError>
    where
        P: FnMut(&str) -> std::result::Result<(&str, T), ParseError>,
        F: FnOnce(T),
    {
        let s: &str = self.str()?;
        let outcome = (parser)(s)?;
        let (_s, t): (&str, T) = outcome;
        save_func(t);
        Ok(self)
    }

    fn parse_put<P, T>(self, mut parser: P, dest: &mut T) -> Result<Self, ParseError>
    where
        P: FnMut(&str) -> std::result::Result<(&str, T), ParseError>,
    {
        let s: &str = self.str()?;
        let outcome = (parser)(s)?;
        let (_s, t): (&str, T) = outcome;
        *dest = t;
        Ok(self)
    }

    fn parse_to_opt<P, T>(self, mut parser: P, dest: &mut Option<T>) -> Result<Self, ParseError>
    where
        P: FnMut(&str) -> std::result::Result<(&str, T), ParseError>,
    {
        let s: &str = self.str()?;
        let outcome = (parser)(s)?;
        let (_s, t): (&str, T) = outcome;
        *dest = Some(t);
        Ok(self)
    }

    fn set_result<T>(self, _t: T) -> Result<(&'a str, T), ParseError> {
        todo!()
    }

    // fn ok<T>(self, t: T) -> Result<(&'a str, T), BadMatch> {
    //     Ok((self.to_str()?, t))
    // }
}

impl<'a> Matchable<'a> for Option<&'a str> {
    type TupleReturn<T> = (Self, T);

    #[inline]
    fn maybe_detuple<T>((s, t): (Self, T)) -> Self::TupleReturn<T> {
        (s, t)
    }

    #[inline]
    fn str(&self) -> Result<&'a str, ParseError> {
        self.ok_or_else(|| error::failure("str on erroring cursor", ""))
    }

    #[inline]
    fn set_str(self, s: &'a str) -> Self {
        Some(s)
    }

    #[inline]
    fn set_error(self, e: ParseError) -> Self {
        trace!("setting (option) error to {e}");
        None
    }

    // type CursorWithSelection = SelectableStr<'a>;
    // type Cursor = Option<&'a str>;
    // type Raw = &'a str;

    // fn selection_start(self) -> Self::CursorWithSelection {
    //     trace!("selection_start({})", formatter(&self));
    //     SelectableStr {
    //         cur: self,
    //         s:   self,
    //         e:   None,
    //         err: None,
    //     }
    // }

    #[inline]
    fn validate(self) -> Result<Self, ParseError> {
        match self.str() {
            Ok(_s) => Ok(self),
            Err(e) => Err(e),
        }
    }
}

// impl<'a> Clone for SelectableStr<'a> {
//     fn clone(&self) -> Self {
//         Self {
//             selection: self.selection.clone(),
//             cur:       self.cur.clone(),
//             err:       None,
//             context:   self.context,
//         }
//     }
// }

impl<'a> Selectable<'a> for Cursor<'a> {
    fn get_selection(&self) -> Result<&'a str, ParseError> {
        if let Some(cur) = self.cur {
            let (s, e) = self.selection.selection(cur);
            let len = s.len() - e.len();
            trace!("get_selection -> '{}'", util::formatter_str(&s[..len]));
            return Ok(&s[..len]);
        }
        if self.err.is_none() {
            dbg!(&self);
        }
        Err(self.clone().err.unwrap())
    }

    fn selection_start(self) -> Self {
        trace!("selection_start({})", util::formatter(&self));
        if let Some(cur) = self.cur {
            Cursor {
                cur: self.cur,
                selection: Selection::Start(cur, None),
                err: self.err,
                context: self.context,
            }
        } else {
            trace!("skipping selection_start");
            self
        }
    }

    fn selection_end(self) -> Self {
        if let Some(_cur) = self.cur {
            trace!(
                "selection_end ({}) => {}",
                self.selection,
                Selection::Start(self.selection.start(), self.cur)
            );
            Self {
                cur: self.cur,
                selection: Selection::Start(self.selection.start(), self.cur),
                err: self.err,
                context: self.context,
            }
        } else {
            trace!("skipping selection_end");
            self
        }
    }
}

impl<'a> Matchable<'a> for Cursor<'a> {
    type TupleReturn<S> = (Self, S);

    fn maybe_detuple<S>((c, s): (Self, S)) -> Self::TupleReturn<S> {
        (c, s)
    }

    #[inline]
    fn str(&self) -> Result<&'a str, ParseError> {
        self.cur.str()
    }

    #[inline]
    fn set_str(self, s: &'a str) -> Self {
        Self {
            selection: self.selection.move_cursor(s),
            cur: self.cur.set_str(s),
            err: self.err,
            context: self.context,
        }
    }

    #[inline]
    fn set_error(self, e: ParseError) -> Self {
        trace!("setting (selection) error to {e}");
        Self {
            selection: self.selection,
            cur: None,
            err: Some(e),
            context: self.context,
        }
    }

    // type CursorWithSelection = Self;
    // type Cursor = Option<&'a str>;
    // type Raw = &'a str;

    // fn selection_start(self) -> Self {
    //     SelectableStr {
    //         cur: self.cur,
    //         s:   self.cur,
    //         e:   None,
    //         err: self.err,
    //     }
    // }

    #[inline]
    fn validate(self) -> Result<Self, ParseError> {
        match self.err {
            None => Ok(self),
            Some(e) => Err(e),
        }
    }
}

impl<'a, T> Selectable<'a> for (Cursor<'a>, T) {
    fn get_selection(&self) -> Result<&'a str, ParseError> {
        self.0.get_selection()
    }

    fn selection_start(self) -> Self {
        (self.0.selection_start(), self.1)
    }

    fn selection_end(self) -> Self {
        (self.0.selection_end(), self.1)
    }
}

impl<'a, T1, T2> Selectable<'a> for (Cursor<'a>, T1, T2) {
    fn get_selection(&self) -> Result<&'a str, ParseError> {
        self.0.get_selection()
    }

    fn selection_start(self) -> Self {
        (self.0.selection_start(), self.1, self.2)
    }

    fn selection_end(self) -> Self {
        (self.0.selection_end(), self.1, self.2)
    }
}

impl<'a, T1, T2, T3> Selectable<'a> for (Cursor<'a>, T1, T2, T3) {
    fn get_selection(&self) -> Result<&'a str, ParseError> {
        self.0.get_selection()
    }

    fn selection_start(self) -> Self {
        (self.0.selection_start(), self.1, self.2, self.3)
    }

    fn selection_end(self) -> Self {
        (self.0.selection_end(), self.1, self.2, self.3)
    }
}

impl<'a, T1, T2> Matchable<'a> for (Cursor<'a>, T1, T2) {
    type TupleReturn<S> = (Cursor<'a>, T1, T2, S);

    fn maybe_detuple<S>((tup, s): (Self, S)) -> Self::TupleReturn<S> {
        let (c, t1, t2) = tup;
        (c, t1, t2, s)
    }

    #[inline]
    fn str(&self) -> Result<&'a str, ParseError> {
        self.0.str()
    }

    #[inline]
    fn set_str(self, s: &'a str) -> Self {
        (self.0.set_str(s), self.1, self.2)
    }

    #[inline]
    fn set_error(self, e: ParseError) -> Self {
        (self.0.set_error(e), self.1, self.2)
    }

    // type CursorWithSelection = Self;
    // type Cursor = Option<&'a str>;
    // type Raw = &'a str;

    // fn selection_start(self) -> Self {
    //     SelectableStr {
    //         cur: self.cur,
    //         s:   self.cur,
    //         e:   None,
    //         err: self.err,
    //     }
    // }
    #[inline]
    fn validate(self) -> Result<Self, ParseError> {
        self.0.validate().map(|c| (c, self.1, self.2))
    }
}

impl<'a, T1, T2, T3> Matchable<'a> for (Cursor<'a>, T1, T2, T3) {
    type TupleReturn<S> = (Cursor<'a>, T1, T2, T3, S);

    fn maybe_detuple<S>((tup, s): (Self, S)) -> Self::TupleReturn<S> {
        let (c, t1, t2, t3) = tup;
        (c, t1, t2, t3, s)
    }

    #[inline]
    fn str(&self) -> Result<&'a str, ParseError> {
        self.0.str()
    }

    #[inline]
    fn set_str(self, s: &'a str) -> Self {
        (self.0.set_str(s), self.1, self.2, self.3)
    }

    #[inline]
    fn set_error(self, e: ParseError) -> Self {
        (self.0.set_error(e), self.1, self.2, self.3)
    }

    #[inline]
    fn validate(self) -> Result<Self, ParseError> {
        self.0.validate().map(|c| (c, self.1, self.2, self.3))
    }
}

impl<'a, T1, T2, T3, T4> Matchable<'a> for (Cursor<'a>, T1, T2, T3, T4) {
    type TupleReturn<S> = (Cursor<'a>, T1, T2, T3, T4, S);

    fn maybe_detuple<S>((tup, s): (Self, S)) -> Self::TupleReturn<S> {
        let (c, t1, t2, t3, t4) = tup;
        (c, t1, t2, t3, t4, s)
    }

    #[inline]
    fn str(&self) -> Result<&'a str, ParseError> {
        self.0.str()
    }

    #[inline]
    fn set_str(self, s: &'a str) -> Self {
        (self.0.set_str(s), self.1, self.2, self.3, self.4)
    }

    #[inline]
    fn set_error(self, e: ParseError) -> Self {
        (self.0.set_error(e), self.1, self.2, self.3, self.4)
    }

    #[inline]
    fn validate(self) -> Result<Self, ParseError> {
        self.0
            .validate()
            .map(|c| (c, self.1, self.2, self.3, self.4))
    }
}

impl<'a, T> Matchable<'a> for (Cursor<'a>, T) {
    type TupleReturn<S> = (Cursor<'a>, T, S);

    fn maybe_detuple<S>((s, t): (Self, S)) -> Self::TupleReturn<S> {
        let (c, s) = s;
        (c, s, t)
    }

    #[inline]
    fn str(&self) -> Result<&'a str, ParseError> {
        self.0.str()
    }

    #[inline]
    fn set_str(self, s: &'a str) -> Self {
        (self.0.set_str(s), self.1)
    }

    #[inline]
    fn set_error(self, e: ParseError) -> Self {
        (self.0.set_error(e), self.1)
    }

    // type CursorWithSelection = Self;
    // type Cursor = Option<&'a str>;
    // type Raw = &'a str;

    // fn selection_start(self) -> Self {
    //     SelectableStr {
    //         cur: self.cur,
    //         s:   self.cur,
    //         e:   None,
    //         err: self.err,
    //     }
    // }

    #[inline]
    fn validate(self) -> Result<Self, ParseError> {
        self.0.validate().map(|c| (c, self.1))
    }
}

#[cfg(test)]
mod tests {

    use std::ops::RangeBounds;

    use crate::text_parser::{cursor, Bind, ParseError, Parser, Selectable};

    use super::{Cursor, Matchable};
    use test_log::test;

    // fn parse_time<C: AsCur>(c: C, f: impl Setter<Instant>) -> Result<C, BadMatch> {
    //     let (hh, mm) = (0, 0);
    //     let c = c
    //         .digits
    // (2..2)
    //         .last(&mut hh)
    //         .text(":")
    //         .digits
    // (2..2)
    //         .last(&mut mm)
    //         .ok()?;
    //     f.set(Time(hh, mm));
    //     c
    // }

    #[derive(PartialEq, Debug)]
    struct Time(i32, i32, f64);

    fn parse_time_v1(s: &str) -> Result<(&str, Time), ParseError> {
        let (mut hh, mut mm, mut sss) = (0_i32, 0_i32, 0_f64);
        let c = cursor(s)
            .digits(2..=2)
            .parse_selection()
            .bind(&mut hh)
            .text(":")
            .digits(2..=2)
            .parse_selection()
            .bind(&mut mm)
            .text(":")
            .select(|c| c.digits(2..=2).text(".").digits(3..=3))
            .parse_selection()
            .bind(&mut sss)
            .validate()?;
        Ok((c.str()?, Time(hh, mm, sss)))
    }

    fn parse_time_v2(s: &str) -> Result<(&str, Time), ParseError> {
        let (mut hh, mut mm, mut sss) = (0_i32, 0_i32, 0_f64);
        let c = cursor(s)
            .digits(2..=2)
            .parse_selection()
            .bind(&mut hh)
            .text(":")
            .digits(2..=2)
            .parse_selection()
            .bind(&mut mm)
            .text(":")
            .selection_start()
            .digits(2..=2)
            .text(".")
            .digits(3..=3)
            .selection_end()
            .parse_selection()
            .bind(&mut sss)
            .validate()?;
        Ok((c.str()?, Time(hh, mm, sss)))
    }

    fn parse_time_v3(s: &str) -> Result<(&str, Time), ParseError> {
        let (c, hh, mm, sss) = cursor(s)
            .digits(2..=2)
            .parse_selection()?
            .text(":")
            .digits(2..=2)
            .parse_selection()?
            .text(":")
            .select(|c| c.digits(2..=2).text(".").digits(3..=3))
            .parse_selection()?;
        Ok((c.str()?, Time(hh, mm, sss)))
    }

    fn parse_time_v4<'a>(s: Cursor<'a>) -> Result<(Cursor<'a>, Time), ParseError> {
        let (c, hh, mm, sss) = s
            .selection_start()
            .digits(2..=2)
            .parse_selection()?
            .text(":")
            .selection_start()
            .digits(2..=2)
            .parse_selection()?
            .text(":")
            .select(|c| c.digits(2..=2).text(".").digits(3..=3))
            .parse_selection()?;
        Ok((c, Time(hh, mm, sss)))
    }

    #[derive(Debug, PartialEq)]
    struct TimePeriod(Time, Time);
    fn parse_time_period(s: &str) -> Result<(&str, TimePeriod), ParseError> {
        let (c, time1, time2) = cursor(s)
            .parse_struct_str(|c| parse_time_v3(c))?
            .text("-")
            .ws()
            .parse_struct_str(|c| parse_time_v3(c))?;
        Ok((c.str()?, TimePeriod(time1, time2)))
    }

    #[test]
    fn test_parse_from_str() {
        let (c, i, j) = cursor("42X45Y")
            .digits(1..)
            .parse_selection::<i32>()
            .unwrap()
            .text("X")
            .digits(1..)
            .parse_selection::<i32>()
            .unwrap();
        assert_eq!(i, 42);
        assert_eq!(j, 45);
        assert_eq!(c.cur, Some("Y"));

        let (c, s) = cursor(" cat ")
            .ws()
            .alphabetics(1..)
            .parse_selection::<String>()
            .unwrap()
            .ws()
            .validate()
            .unwrap();
        assert_eq!(s, String::from("cat"));
        assert_eq!(c.cur, Some(""));

        let (c, s) = cursor(" cat ")
            .ws()
            .alphabetics(1..)
            .parse_selection::<String>()
            .unwrap()
            .ws()
            .validate()
            .unwrap();
        assert_eq!(s, String::from("cat"));
        assert_eq!(c.cur, Some(""));
    }

    #[test]
    fn test_examples() {
        // impl<'p, T> Parser<'a, T, SelectableStr<'p>> for for<'a> fn(SelectableStr<'a>) -> Result<(SelectableStr<'a>,T), BadMatch> {
        //     fn parse(&self, _s: SelectableStr<'p>) -> Result<(SelectableStr<'p>, T), BadMatch> {
        //         todo!()
        //     }
        // }

        fn lp<'a, T>(s: Cursor<'a>, mut p: impl Parser<'a, T>) {
            let _ = p.parse(s);
        }

        #[derive(Default)]
        struct StructEx;
        impl StructEx {
            fn parse_ex2<'a>(&self, s: &'a str) -> Result<(&'a str, String), ParseError> {
                Ok((s, String::from("Example2")))
            }
            fn parse_ex4<'a>(&self, s: Cursor<'a>) -> Result<(Cursor<'a>, String), ParseError> {
                Ok((s, String::from("Example4")))
            }
        }

        fn parse_ex1(s: &str) -> Result<(&str, String), ParseError> {
            Ok((s, String::from("Example1")))
        }

        fn parse_ex3(s: Cursor) -> Result<(Cursor, String), ParseError> {
            Ok((s, String::from("Example3")))
        }

        type StrFunc<T> = for<'a> fn(&'a str) -> Result<(&'a str, T), ParseError>;
        type StrMethod<X, T> = for<'a> fn(x: &'a X, &'a str) -> Result<(&'a str, T), ParseError>;
        type CurFunc<T> = for<'a> fn(Cursor<'a>) -> Result<(Cursor<'a>, T), ParseError>;

        // fn cursorify<'a, T>(
        //      f: &'a mut impl FnMut(&'a str) -> Result<(&'a str, T), BadMatch>,
        // ) -> impl FnMut(SelectableStr<'a>) -> Result<(SelectableStr<'a>, T), BadMatch>   {
        //     |c: SelectableStr<'a>| { (f)(c.str()?).map(|(s,t)| (cursor(s),t) ) }
        // }

        let selfie = StructEx;

        let (_c, _vec) = cursor("")
            .parse_struct_vec(|c| c.ws().parse_selection::<i32>())
            .unwrap();

        // let (_c, _vec) = cursor("11:20:24.123")
        //     .parse_struct_vec(cursorify(|c| parse_time_v2(c)))
        //     .unwrap();

        // let (_c, _vec) = cursor("")
        //     .parse_struct_vec(cursorify(|c| selfie.parse_ex2(c)))
        //     .unwrap();

        let _f: StrMethod<StructEx, String> = StructEx::parse_ex2;
        let f: for<'a> fn(&'a str) -> Result<(&'a str, String), ParseError> = parse_ex1;
        let tup = (&selfie, StructEx::parse_ex2 as StrMethod<_, _>);
        lp(cursor("parse_ex1 as ...     "), f);
        lp(cursor("parse_ex1 as StrFunc "), parse_ex1 as StrFunc<_>);
        lp(cursor("(&selfie,f)          "), tup);
        lp(cursor("|c| self.parse_ex4(c)"), |c| selfie.parse_ex4(c));
        lp(cursor("parse_ex3            "), parse_ex3);
        lp(cursor("|c| parse_ex(c)      "), |c| parse_ex3(c));
        lp(cursor("11:20:24.123         "), |c| parse_time_v4(c));
        lp(cursor("11:21:24.123         "), parse_time_v4);

        // https://internals.rust-lang.org/t/extending-implicit-coercion-of-function-items-to-function-pointers-during-trait-resolution/17093
        lp(cursor("11:22:24.123         "), parse_time_v3 as StrFunc<_>);
        lp(cursor("11:23:24.123         "), parse_time_v2 as StrFunc<_>);
    }

    #[test]
    fn test_parse_range() {
        fn rb<R: RangeBounds<i32>>(_: R) {}
        rb(1..3);
        rb(..=3);
        rb(..);
    }

    #[test]
    fn test_parse_nested() {
        fn rb<R: RangeBounds<i32>>(_: R) {}
        rb(1..3);
        rb(..=3);
        rb(..);

        assert_eq!(
            parse_time_v1("23:59:13.234").unwrap(),
            ("", Time(23, 59, 13.234))
        );
        assert_eq!(
            parse_time_v2("23:59:13.234").unwrap(),
            ("", Time(23, 59, 13.234))
        );
        assert_eq!(
            parse_time_v3("23:59:13.234").unwrap(),
            ("", Time(23, 59, 13.234))
        );
        assert_eq!(
            parse_time_v4(cursor("23:59:13.234")).unwrap().1,
            Time(23, 59, 13.234)
        );

        assert_eq!(
            parse_time_v1("23:59:13.234Hello").unwrap(),
            ("Hello", Time(23, 59, 13.234))
        );
        assert_eq!(parse_time_v3("23:X:13.234Hello").is_err(), true);
        assert_eq!(
            parse_time_period("13:12:13.234-12:34:19.234-rest").unwrap(),
            (
                "-rest",
                TimePeriod(Time(13, 12, 13.234), Time(12, 34, 19.234))
            )
        );
    }

    #[test]
    fn test_parse_lists() {
        let s = cursor("1,2,3,4,5,");
        let mut vec1 = vec![];
        let res1 = s.parse_struct_vec_to(
            |c| {
                c.selection_start()
                    .digits(1..5)
                    .selection_end()
                    .text_alt(&[",", " "])
                    .parse_selection_as_str()
            },
            &mut vec1,
        );
        assert_eq!(res1.is_ok(), true);
        assert_eq!(vec1.len(), 5, "vec:{:?}", vec1);
        assert_eq!(vec1[0], "1");
        assert_eq!(vec1[1], "2");
        assert_eq!(res1.unwrap().cur, Some(""));

        let mut ll2: Vec<i32> = Vec::new();
        let s = cursor("{1,2,3,4,5,}");
        let res2 = s
            .debug_context("array")
            .text("{")
            .repeat(1.., |c| c.digits(1..).append_last(&mut ll2).maybe(","))
            .text("}")
            .validate();
        assert_eq!(res2.is_ok(), true);
        assert_eq!(ll2.len(), 5, "linkedlist:{:?}", ll2);

        fn parse_str_time_array(s: &str) -> Result<(&str, Vec<Time>), ParseError> {
            let (c, vec) = cursor(s)
                .debug_context("str time array")
                .text("{")
                .ws()
                .parse_struct_vec(|c| {
                    c.parse_struct_str(|c| parse_time_v3(c))?
                        .maybe(",")
                        .ws()
                        .validate()
                })?
                .ws()
                .text("}")
                .validate()?;
            Ok((c.str()?, vec))
        }
        let res = parse_str_time_array("{01:02:03.345, 02:02:03.346, 23:02:03.347}").unwrap();
        assert_eq!(res.1.len(), 3);
        assert_eq!(res.1[0], Time(1, 2, 3.345));
        assert_eq!(res.1[2], Time(23, 2, 3.347));
        assert_eq!(res.1.len(), 3);
        assert_eq!(res.0, "");

        fn parse_time_array(s: Cursor) -> Result<(Cursor, Vec<Time>), ParseError> {
            let (c, vec) = s
                .debug_context("time array")
                .text("{")
                .ws()
                .parse_struct_vec(|c| parse_time_v4(c)?.maybe(",").ws().validate())?
                .ws()
                .text("}")
                .validate()?;
            Ok((c, vec))
        }
        let res = parse_time_array(cursor("{01:02:03.345, 02:02:03.346, 23:02:03.347}")).unwrap();
        assert_eq!(res.1.len(), 3);
        assert_eq!(res.1[0], Time(1, 2, 3.345));
        assert_eq!(res.1[2], Time(23, 2, 3.347));
        assert_eq!(res.1.len(), 3);
        assert_eq!(res.0.str().unwrap(), "");
    }
}

//     assert_eq!(
//         parse_hour_mins_v1("23:59a").unwrap(),
//         ("a", HourMin(23, 59))
//     );
//     // assert_eq!(parse_hour_mins("blob").is_err(), true);
//     // let mut ll3: Vec<i32> = Vec::new();
//     // let s = cursor("[11:23, 09:15, 15:23]");
//     // let res3 = s
//     //     .text("[")
//     //     .repeat(|c| parse_time(c, &mut ll3).maybe(",").ws())
//     //     .text("]")
//     //     .parse();
//     // assert_eq!(res2.is_ok(), true);

//     // let res2 = s + "{" + repeat(|c| c + digits(1..) >> &mut ll2 + maybe(",")) + "}" ;

//     // use crate::text_parser::SelectableCursor;
//     // fn apply() -> anyhow::Result<()> {
//     //     let c = Some("test").selection_start();
//     //     let (c1, o2) = c
//     //         .parse_selection_as_f64()
//     //         .else_parse(|_| c.parse_selection_as_f64())?;
//     //     Ok(())
//     // }
//     // cur.next_word()
//     // cur.ws()
//     // let c = cur.next_parse_i32()?;
//     // cur.
//     // let c = {
//     //     if let Some(c1) = cur.next_parse_i32().match_some()?;
//     //       c1;
//     //     else {
//     //       cur.next_parse_i32()?;
//     //     }
// }
// cur.next_word()
// cur.ws()
// let c = cur.next_parse_i32()?;
// cur.
// let c = {
//     if let Some(c1) = cur.next_parse_i32().match_some()?;
//       c1;
//     else {
//       cur.next_parse_i32()?;
//     }
