/// String utility functions

/// Format a number with thousands separators
pub fn format_number_with_separators(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::new();
    let chars: Vec<char> = s.chars().collect();
    
    for (i, ch) in chars.iter().enumerate() {
        if i > 0 && (chars.len() - i) % 3 == 0 {
            result.push(',');
        }
        result.push(*ch);
    }
    
    result
}

/// Format a coin amount with thousands separators.
/// e.g. `24000000` → `"24,000,000"`, `-500000` → `"-500,000"`
pub fn format_coins(amount: i64) -> String {
    let negative = amount < 0;
    let abs = amount.unsigned_abs();
    let s = abs.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    let formatted: String = result.chars().rev().collect();
    if negative { format!("-{}", formatted) } else { formatted }
}

/// Format an f64 coin amount with comma separators, preserving one decimal
/// digit when the fractional part is non-zero (e.g. 600000.5 → "600,000.5").
pub fn format_coins_f64(amount: f64) -> String {
    let tenths = (amount * 10.0).round() as i64;
    let int_part = tenths / 10;
    let frac_digit = (tenths % 10).abs();
    let int_str = format_coins(int_part);
    if frac_digit == 0 {
        int_str
    } else {
        format!("{}.{}", int_str, frac_digit)
    }
}

/// Calculate Hypixel AH fee based on price tier.
/// - <10M  → 1%
/// - <100M → 2%
/// - ≥100M → 2.5%
pub fn calculate_ah_fee(price: u64) -> u64 {
    if price < 10_000_000 {
        price / 100
    } else if price < 100_000_000 {
        price * 2 / 100
    } else {
        price * 25 / 1000
    }
}

/// Remove Minecraft color codes from text
/// Format: §x where x is a color code
pub fn remove_minecraft_colors(text: &str) -> String {
    let mut result = String::new();
    let mut chars = text.chars();
    
    while let Some(ch) = chars.next() {
        if ch == '§' || ch == '┬' {
            // Skip the next character (color code)
            chars.next();
        } else {
            result.push(ch);
        }
    }
    
    result
}

/// Convert string to title case
pub fn to_title_case(s: &str) -> String {
    s.split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => {
                    first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                }
            }
        })
        .collect::<Vec<String>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_number_with_separators() {
        assert_eq!(format_number_with_separators(1000), "1,000");
        assert_eq!(format_number_with_separators(1000000), "1,000,000");
        assert_eq!(format_number_with_separators(123), "123");
    }

    #[test]
    fn test_remove_minecraft_colors() {
        assert_eq!(
            remove_minecraft_colors("§aGreen§r Text"),
            "Green Text"
        );
        assert_eq!(
            remove_minecraft_colors("§6Buy Item Right Now"),
            "Buy Item Right Now"
        );
    }

    #[test]
    fn test_to_title_case() {
        assert_eq!(to_title_case("hello world"), "Hello World");
        assert_eq!(to_title_case("HELLO WORLD"), "Hello World");
        assert_eq!(to_title_case("hello"), "Hello");
    }
}
