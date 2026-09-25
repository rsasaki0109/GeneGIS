//! Minimal WKT reader and writer for CSV geometry columns.

use geo_types::{
    Coord, Geometry, GeometryCollection, LineString, MultiLineString, MultiPoint, MultiPolygon,
    Point, Polygon,
};

use crate::error::{Result, ToolkitError};

/// Format a geometry as WKT.
pub fn write(geometry: &Geometry<f64>) -> String {
    fn coords(line: &LineString<f64>) -> String {
        line.0
            .iter()
            .map(|c| format!("{} {}", num(c.x), num(c.y)))
            .collect::<Vec<_>>()
            .join(", ")
    }
    fn polygon(p: &Polygon<f64>) -> String {
        std::iter::once(p.exterior())
            .chain(p.interiors())
            .map(|ring| format!("({})", coords(ring)))
            .collect::<Vec<_>>()
            .join(", ")
    }
    match geometry {
        Geometry::Point(p) => format!("POINT ({} {})", num(p.x()), num(p.y())),
        Geometry::LineString(l) => format!("LINESTRING ({})", coords(l)),
        Geometry::Line(l) => write(&Geometry::LineString(LineString(vec![l.start, l.end]))),
        Geometry::Polygon(p) => format!("POLYGON ({})", polygon(p)),
        Geometry::Rect(r) => write(&Geometry::Polygon(r.to_polygon())),
        Geometry::Triangle(t) => write(&Geometry::Polygon(t.to_polygon())),
        Geometry::MultiPoint(m) => format!(
            "MULTIPOINT ({})",
            m.0.iter()
                .map(|p| format!("({} {})", num(p.x()), num(p.y())))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Geometry::MultiLineString(m) => format!(
            "MULTILINESTRING ({})",
            m.0.iter()
                .map(|l| format!("({})", coords(l)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Geometry::MultiPolygon(m) => format!(
            "MULTIPOLYGON ({})",
            m.0.iter()
                .map(|p| format!("({})", polygon(p)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Geometry::GeometryCollection(c) => format!(
            "GEOMETRYCOLLECTION ({})",
            c.0.iter().map(write).collect::<Vec<_>>().join(", ")
        ),
    }
}

fn num(v: f64) -> String {
    let s = format!("{v}");
    s
}

/// Parse WKT (Z/M ordinates are dropped). An optional `SRID=n;` prefix is ignored.
pub fn read(text: &str) -> Result<Geometry<f64>> {
    let text = match text.trim().split_once(';') {
        Some((prefix, rest)) if prefix.trim().to_ascii_uppercase().starts_with("SRID=") => rest,
        _ => text,
    };
    let mut parser = Parser {
        tokens: tokenize(text)?,
        pos: 0,
    };
    let geometry = parser.geometry()?;
    if parser.pos != parser.tokens.len() {
        return Err(err("trailing characters after geometry"));
    }
    Ok(geometry)
}

fn err(reason: impl Into<String>) -> ToolkitError {
    ToolkitError::import("wkt", reason)
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Word(String),
    Number(f64),
    Open,
    Close,
    Comma,
}

fn tokenize(text: &str) -> Result<Vec<Token>> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' | '\n' | '\r' => i += 1,
            '(' => {
                tokens.push(Token::Open);
                i += 1;
            }
            ')' => {
                tokens.push(Token::Close);
                i += 1;
            }
            ',' => {
                tokens.push(Token::Comma);
                i += 1;
            }
            c if c.is_ascii_alphabetic() => {
                let start = i;
                while i < chars.len() && chars[i].is_ascii_alphabetic() {
                    i += 1;
                }
                tokens.push(Token::Word(
                    chars[start..i]
                        .iter()
                        .collect::<String>()
                        .to_ascii_uppercase(),
                ));
            }
            c if c.is_ascii_digit() || c == '-' || c == '+' || c == '.' => {
                let start = i;
                i += 1;
                while i < chars.len()
                    && (chars[i].is_ascii_digit()
                        || matches!(chars[i], '.' | 'e' | 'E')
                        || (matches!(chars[i], '-' | '+') && matches!(chars[i - 1], 'e' | 'E')))
                {
                    i += 1;
                }
                let literal: String = chars[start..i].iter().collect();
                let value = literal
                    .parse::<f64>()
                    .map_err(|_| err(format!("invalid number {literal}")))?;
                tokens.push(Token::Number(value));
            }
            other => return Err(err(format!("unexpected character {other:?}"))),
        }
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        token
    }

    fn expect(&mut self, token: Token) -> Result<()> {
        match self.next() {
            Some(t) if t == token => Ok(()),
            other => Err(err(format!("expected {token:?}, found {other:?}"))),
        }
    }

    /// Consume dimension qualifiers (Z, M, ZM) and report whether EMPTY follows.
    fn qualifiers(&mut self) -> bool {
        while let Some(Token::Word(word)) = self.peek() {
            match word.as_str() {
                "Z" | "M" | "ZM" => self.pos += 1,
                "EMPTY" => {
                    self.pos += 1;
                    return true;
                }
                _ => break,
            }
        }
        false
    }

    fn coord(&mut self) -> Result<Coord<f64>> {
        let mut values = Vec::new();
        while let Some(Token::Number(v)) = self.peek() {
            values.push(*v);
            self.pos += 1;
        }
        if values.len() < 2 {
            return Err(err("coordinate needs at least x and y"));
        }
        Ok(Coord {
            x: values[0],
            y: values[1],
        })
    }

    fn list<T>(&mut self, mut item: impl FnMut(&mut Self) -> Result<T>) -> Result<Vec<T>> {
        self.expect(Token::Open)?;
        let mut out = vec![item(self)?];
        while self.peek() == Some(&Token::Comma) {
            self.pos += 1;
            out.push(item(self)?);
        }
        self.expect(Token::Close)?;
        Ok(out)
    }

    fn line(&mut self) -> Result<LineString<f64>> {
        Ok(LineString(self.list(Self::coord)?))
    }

    fn polygon(&mut self) -> Result<Polygon<f64>> {
        let mut rings = self.list(Self::line)?;
        let exterior = rings.remove(0);
        Ok(Polygon::new(exterior, rings))
    }

    fn multipoint_member(&mut self) -> Result<Point<f64>> {
        if self.peek() == Some(&Token::Open) {
            self.pos += 1;
            let c = self.coord()?;
            self.expect(Token::Close)?;
            Ok(Point(c))
        } else {
            Ok(Point(self.coord()?))
        }
    }

    fn geometry(&mut self) -> Result<Geometry<f64>> {
        let Some(Token::Word(kind)) = self.next() else {
            return Err(err("expected geometry keyword"));
        };
        let empty = self.qualifiers();
        Ok(match kind.as_str() {
            "POINT" if empty => Geometry::MultiPoint(MultiPoint(vec![])),
            "POINT" => {
                self.expect(Token::Open)?;
                let c = self.coord()?;
                self.expect(Token::Close)?;
                Geometry::Point(Point(c))
            }
            "LINESTRING" if empty => Geometry::LineString(LineString(vec![])),
            "LINESTRING" => Geometry::LineString(self.line()?),
            "POLYGON" if empty => Geometry::MultiPolygon(MultiPolygon(vec![])),
            "POLYGON" => Geometry::Polygon(self.polygon()?),
            "MULTIPOINT" if empty => Geometry::MultiPoint(MultiPoint(vec![])),
            "MULTIPOINT" => Geometry::MultiPoint(MultiPoint(self.list(Self::multipoint_member)?)),
            "MULTILINESTRING" if empty => Geometry::MultiLineString(MultiLineString(vec![])),
            "MULTILINESTRING" => Geometry::MultiLineString(MultiLineString(self.list(Self::line)?)),
            "MULTIPOLYGON" if empty => Geometry::MultiPolygon(MultiPolygon(vec![])),
            "MULTIPOLYGON" => Geometry::MultiPolygon(MultiPolygon(self.list(Self::polygon)?)),
            "GEOMETRYCOLLECTION" if empty => {
                Geometry::GeometryCollection(GeometryCollection(vec![]))
            }
            "GEOMETRYCOLLECTION" => {
                Geometry::GeometryCollection(GeometryCollection(self.list(Self::geometry)?))
            }
            other => return Err(err(format!("unsupported geometry type {other}"))),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_writes_round_trip() {
        for text in [
            "POINT (136.9 35.1)",
            "LINESTRING (0 0, 1 1, 2 0)",
            "POLYGON ((0 0, 4 0, 4 4, 0 0), (1 1, 2 1, 2 2, 1 1))",
            "MULTIPOINT ((1 2), (3 4))",
            "MULTIPOLYGON (((0 0, 1 0, 1 1, 0 0)), ((5 5, 6 5, 6 6, 5 5)))",
        ] {
            let geometry = read(text).unwrap();
            assert_eq!(read(&write(&geometry)).unwrap(), geometry, "{text}");
        }
    }

    #[test]
    fn accepts_z_srid_and_bare_multipoint() {
        assert_eq!(
            read("SRID=4326;POINT Z (1 2 3)").unwrap(),
            Geometry::Point(Point::new(1.0, 2.0))
        );
        assert_eq!(
            read("MULTIPOINT (1 2, 3 4)").unwrap(),
            Geometry::MultiPoint(MultiPoint(vec![Point::new(1.0, 2.0), Point::new(3.0, 4.0)]))
        );
        assert_eq!(
            read("point(1e3 -2.5E-1)").unwrap(),
            Geometry::Point(Point::new(1000.0, -0.25))
        );
    }

    #[test]
    fn rejects_garbage() {
        assert!(read("POINT (1)").is_err());
        assert!(read("CIRCLE (1 2)").is_err());
        assert!(read("POINT (1 2) extra").is_err());
    }
}
