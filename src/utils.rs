pub fn rus_numeric<'a>(n: usize, zero: &'a str, one: &'a str, two: &'a str) -> &'a str {
    match n % 100 {
        11..=20 => zero,
        _ => match n % 10 {
            1 => one,
            2..=4 => two,
            _ => zero,
        },
    }
}

pub fn escape_telegram_symbols(str: &str, symbols: &str) -> String {
    let mut result = String::new();
    let chars = symbols.chars().collect::<Vec<_>>();
    for c in str.chars() {
        if chars.contains(&c) {
            result.push('\\');
        }
        result.push(c);
    }
    result
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_numeric() {
        assert_eq!(rus_numeric(0, "яблок", "яблоко", "яблока"), "яблок");
        assert_eq!(rus_numeric(1, "яблок", "яблоко", "яблока"), "яблоко");
        assert_eq!(rus_numeric(2, "яблок", "яблоко", "яблока"), "яблока");
        assert_eq!(rus_numeric(3, "яблок", "яблоко", "яблока"), "яблока");
        assert_eq!(rus_numeric(4, "яблок", "яблоко", "яблока"), "яблока");
        assert_eq!(rus_numeric(5, "яблок", "яблоко", "яблока"), "яблок");
        assert_eq!(rus_numeric(10, "яблок", "яблоко", "яблока"), "яблок");
        assert_eq!(rus_numeric(11, "яблок", "яблоко", "яблока"), "яблок");
        assert_eq!(rus_numeric(20, "яблок", "яблоко", "яблока"), "яблок");
        assert_eq!(rus_numeric(21, "яблок", "яблоко", "яблока"), "яблоко");
        assert_eq!(rus_numeric(22, "яблок", "яблоко", "яблока"), "яблока");
        assert_eq!(rus_numeric(25, "яблок", "яблоко", "яблока"), "яблок");
        assert_eq!(rus_numeric(10031, "яблок", "яблоко", "яблока"), "яблоко");
    }
}
