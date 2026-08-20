use crate::model::{Action, Query, ResultItem};

use super::{Provider, ProviderContext, ProviderOutput};

const PROVIDER_ID: &str = "calculator";
const MAX_EXPRESSION_BYTES: usize = 256;
const MAX_PARSE_DEPTH: usize = 32;

pub(crate) struct CalculatorProvider;

impl Provider for CalculatorProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn query(&self, query: &Query, _context: &ProviderContext<'_>) -> ProviderOutput {
        let Some(expression) = query.raw().strip_prefix('=') else {
            return ProviderOutput::empty();
        };
        let expression = expression.trim();
        if expression.is_empty() {
            return calculator_message("计算器：输入 = 表达式，例如 = (12+3)*4");
        }
        let Ok(value) = evaluate(expression) else {
            return calculator_message("计算表达式无效，请检查数字、括号和运算符");
        };
        let result = format_number(value);
        ProviderOutput {
            results: vec![ResultItem {
                provider_id: PROVIDER_ID,
                title: format!("= {result}"),
                score: 200_000,
                actions: vec![Action::CopyText { text: result }],
            }],
            exclusive: true,
        }
    }
}

fn calculator_message(title: &str) -> ProviderOutput {
    ProviderOutput {
        results: vec![ResultItem {
            provider_id: PROVIDER_ID,
            title: title.to_owned(),
            score: 200_000,
            actions: Vec::new(),
        }],
        exclusive: true,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CalculationError {
    Empty,
    InvalidNumber,
    UnexpectedToken,
    MissingClosingParenthesis,
    DivisionByZero,
    NonFinite,
    TooComplex,
}

fn evaluate(expression: &str) -> Result<f64, CalculationError> {
    if expression.is_empty() {
        return Err(CalculationError::Empty);
    }
    if expression.len() > MAX_EXPRESSION_BYTES {
        return Err(CalculationError::TooComplex);
    }
    let mut parser = Parser::new(expression);
    let value = parser.parse_expression(0)?;
    parser.skip_whitespace();
    if parser.position != parser.input.len() {
        return Err(CalculationError::UnexpectedToken);
    }
    if !value.is_finite() {
        return Err(CalculationError::NonFinite);
    }
    Ok(value)
}

pub(crate) fn format_number(value: f64) -> String {
    if value == 0.0 {
        return "0".to_owned();
    }
    let rounded = if value.abs() < 1_000_000_000_000.0 {
        (value * 1_000_000_000_000.0).round() / 1_000_000_000_000.0
    } else {
        value
    };
    rounded.to_string()
}

struct Parser<'a> {
    input: &'a [u8],
    position: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input: input.as_bytes(),
            position: 0,
        }
    }

    fn parse_expression(&mut self, depth: usize) -> Result<f64, CalculationError> {
        let mut value = self.parse_term(depth)?;
        loop {
            self.skip_whitespace();
            match self.peek() {
                Some(b'+') => {
                    self.position += 1;
                    value += self.parse_term(depth)?;
                }
                Some(b'-') => {
                    self.position += 1;
                    value -= self.parse_term(depth)?;
                }
                _ => return Ok(value),
            }
            if !value.is_finite() {
                return Err(CalculationError::NonFinite);
            }
        }
    }

    fn parse_term(&mut self, depth: usize) -> Result<f64, CalculationError> {
        let mut value = self.parse_unary(depth)?;
        loop {
            self.skip_whitespace();
            let operator = match self.peek() {
                Some(operator @ (b'*' | b'/' | b'%')) => operator,
                _ => return Ok(value),
            };
            self.position += 1;
            let right = self.parse_unary(depth)?;
            value = match operator {
                b'*' => value * right,
                b'/' => {
                    if right == 0.0 {
                        return Err(CalculationError::DivisionByZero);
                    }
                    value / right
                }
                b'%' => {
                    if right == 0.0 {
                        return Err(CalculationError::DivisionByZero);
                    }
                    value % right
                }
                _ => unreachable!(),
            };
            if !value.is_finite() {
                return Err(CalculationError::NonFinite);
            }
        }
    }

    fn parse_unary(&mut self, depth: usize) -> Result<f64, CalculationError> {
        if depth > MAX_PARSE_DEPTH {
            return Err(CalculationError::TooComplex);
        }
        self.skip_whitespace();
        match self.peek() {
            Some(b'+') => {
                self.position += 1;
                self.parse_unary(depth + 1)
            }
            Some(b'-') => {
                self.position += 1;
                Ok(-self.parse_unary(depth + 1)?)
            }
            _ => self.parse_primary(depth),
        }
    }

    fn parse_primary(&mut self, depth: usize) -> Result<f64, CalculationError> {
        self.skip_whitespace();
        if self.peek() == Some(b'(') {
            if depth >= MAX_PARSE_DEPTH {
                return Err(CalculationError::TooComplex);
            }
            self.position += 1;
            let value = self.parse_expression(depth + 1)?;
            self.skip_whitespace();
            if self.peek() != Some(b')') {
                return Err(CalculationError::MissingClosingParenthesis);
            }
            self.position += 1;
            return Ok(value);
        }
        self.parse_number()
    }

    fn parse_number(&mut self) -> Result<f64, CalculationError> {
        self.skip_whitespace();
        let start = self.position;
        let mut digits = 0;
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.position += 1;
            digits += 1;
        }
        if self.peek() == Some(b'.') {
            self.position += 1;
            while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.position += 1;
                digits += 1;
            }
        }
        if digits == 0 {
            return Err(CalculationError::InvalidNumber);
        }
        let text = std::str::from_utf8(&self.input[start..self.position])
            .map_err(|_| CalculationError::InvalidNumber)?;
        text.parse().map_err(|_| CalculationError::InvalidNumber)
    }

    fn skip_whitespace(&mut self) {
        while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.position += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.position).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::{CalculationError, evaluate, format_number};

    #[test]
    fn respects_precedence_parentheses_and_unary_operators() {
        assert_eq!(evaluate("2 + 3 * 4"), Ok(14.0));
        assert_eq!(evaluate("(2 + 3) * 4"), Ok(20.0));
        assert_eq!(evaluate("-(2 + 3) * -2"), Ok(10.0));
        assert_eq!(evaluate("17 % 5"), Ok(2.0));
    }

    #[test]
    fn supports_decimal_numbers_and_readable_formatting() {
        assert_eq!(evaluate(".5 + 1.25"), Ok(1.75));
        assert_eq!(format_number(evaluate("0.1 + 0.2").unwrap()), "0.3");
        assert_eq!(format_number(evaluate("10 / 4").unwrap()), "2.5");
        assert_eq!(format_number(-0.0), "0");
    }

    #[test]
    fn rejects_invalid_or_unsafe_expressions() {
        assert_eq!(evaluate("1 / 0"), Err(CalculationError::DivisionByZero));
        assert_eq!(evaluate("2 + nope"), Err(CalculationError::InvalidNumber));
        assert_eq!(
            evaluate("2 + 3 extra"),
            Err(CalculationError::UnexpectedToken)
        );
        assert_eq!(
            evaluate("(((((((((((((((((((((((((((((((((1)))))))))))))))))))))))))))))))))"),
            Err(CalculationError::TooComplex)
        );
    }
}
