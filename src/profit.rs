use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// A single profit data point: (unix_timestamp_secs, cumulative_profit_coins)
pub type ProfitPoint = (u64, i64);

/// Thread-safe profit tracker for AH and Bazaar realized profits.
pub struct ProfitTracker {
    inner: Mutex<ProfitTrackerInner>,
}

struct ProfitTrackerInner {
    ah_points: Vec<ProfitPoint>,
    bz_points: Vec<ProfitPoint>,
    ah_total: i64,
    bz_total: i64,
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl ProfitTracker {
    pub fn new() -> Self {
        let now = now_unix();
        Self {
            inner: Mutex::new(ProfitTrackerInner {
                ah_points: vec![(now, 0)],
                bz_points: vec![(now, 0)],
                ah_total: 0,
                bz_total: 0,
            }),
        }
    }

    /// Record a realized AH profit (positive or negative).
    pub fn record_ah_profit(&self, profit: i64) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.ah_total += profit;
            let total = inner.ah_total;
            inner.ah_points.push((now_unix(), total));
        }
    }

    /// Replace the AH total with an authoritative value (e.g. from Coflnet
    /// `/cofl profit`) and record a new data-point so the chart updates.
    pub fn set_ah_total(&self, total: i64) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.ah_total = total;
            inner.ah_points.push((now_unix(), total));
        }
    }

    /// Record a realized Bazaar profit (positive or negative).
    pub fn record_bz_profit(&self, profit: i64) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.bz_total += profit;
            let total = inner.bz_total;
            inner.bz_points.push((now_unix(), total));
        }
    }

    /// Replace the BZ total with an authoritative value (e.g. from `/cofl bz l`
    /// accumulated profit) and record a new data-point so the chart updates.
    pub fn set_bz_total(&self, total: i64) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.bz_total = total;
            inner.bz_points.push((now_unix(), total));
        }
    }

    /// Get all AH profit data points.
    pub fn ah_points(&self) -> Vec<ProfitPoint> {
        self.inner
            .lock()
            .map(|i| i.ah_points.clone())
            .unwrap_or_default()
    }

    /// Get all Bazaar profit data points.
    pub fn bz_points(&self) -> Vec<ProfitPoint> {
        self.inner
            .lock()
            .map(|i| i.bz_points.clone())
            .unwrap_or_default()
    }

    /// Get totals: (ah_total, bz_total)
    pub fn totals(&self) -> (i64, i64) {
        self.inner
            .lock()
            .map(|i| (i.ah_total, i.bz_total))
            .unwrap_or((0, 0))
    }
}

// ---------------------------------------------------------------------------
// Chat message parsers for Coflnet profit responses
// ---------------------------------------------------------------------------

/// Parse a human-readable short number like `82.7M`, `1.5B`, `250K`, or `500`.
pub fn parse_short_number(s: &str) -> Option<i64> {
    let s = s.replace(',', "");
    let (num_part, multiplier) = if let Some(n) = s.strip_suffix('B').or_else(|| s.strip_suffix('b')) {
        (n, 1_000_000_000f64)
    } else if let Some(n) = s.strip_suffix('M').or_else(|| s.strip_suffix('m')) {
        (n, 1_000_000f64)
    } else if let Some(n) = s.strip_suffix('K').or_else(|| s.strip_suffix('k')) {
        (n, 1_000f64)
    } else {
        (s.as_str(), 1f64)
    };
    let val: f64 = num_part.parse().ok()?;
    Some((val * multiplier) as i64)
}

/// Parse a Coflnet `/cofl profit` response and return the total profit in coins.
///
/// Expected format (color-stripped):
/// `"According to our data <ign> made <amount> in the last <days> days across <N> auctions"`
pub fn parse_cofl_profit_response(clean_msg: &str) -> Option<i64> {
    let rest = clean_msg.strip_prefix("According to our data ")?;
    let made_idx = rest.find(" made ")?;
    let after_made = &rest[made_idx + 6..];
    let end = after_made.find(" in the last ")?;
    let amount_str = after_made[..end].trim();
    parse_short_number(amount_str)
}

/// Parse a single flip line from `/cofl bz l` output and return the profit.
///
/// Expected format: `"2xJungle Key: 1.05M -> 287K => -768K(1)"`
pub fn parse_bz_list_flip_profit(line: &str) -> Option<i64> {
    let arrow_idx = line.find("=> ")?;
    let after_arrow = &line[arrow_idx + 3..];
    let paren_idx = after_arrow.find('(')?;
    let profit_str = after_arrow[..paren_idx].trim();
    parse_short_number(profit_str)
}

/// Parse a single flip line from `/cofl bz l` and return `(item_name, profit, flip_count)`.
///
/// Expected format: `"128xWorm Membrane: 7.16M -> 7.91M => 741K(7)"`
pub fn parse_bz_list_flip_detail(line: &str) -> Option<(String, i64, u32)> {
    let x_idx = line.find('x')?;
    let amount_str = line[..x_idx].trim();
    if amount_str.is_empty() || !amount_str.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let rest = &line[x_idx + 1..];
    let colon_idx = rest.find(':')?;
    let item_name = rest[..colon_idx].trim().to_string();
    if item_name.is_empty() {
        return None;
    }
    let arrow_idx = rest.find("=> ")?;
    let after_arrow = &rest[arrow_idx + 3..];
    let paren_idx = after_arrow.find('(')?;
    let profit_str = after_arrow[..paren_idx].trim();
    let profit = parse_short_number(profit_str)?;
    let after_paren = &after_arrow[paren_idx + 1..];
    let close_paren = after_paren.find(')')?;
    let count: u32 = after_paren[..close_paren].trim().parse().ok()?;
    Some((item_name, profit, count))
}

/// Parse a Coflnet `/cofl bz h` response and return the total profit in coins.
///
/// Looks for `"Total Profit: "` in the message and parses the value after it.
pub fn parse_cofl_bz_h_total_profit(clean_msg: &str) -> Option<i64> {
    let prefix = "Total Profit: ";
    let idx = clean_msg.find(prefix)?;
    let after = &clean_msg[idx + prefix.len()..];
    let value_str: String = after.chars().take_while(|c| !c.is_whitespace()).collect();
    parse_short_number(&value_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cofl_profit_response_82m() {
        let msg = "According to our data TestPlayer made 82.7M in the last 7 days across 10 auctions";
        assert_eq!(parse_cofl_profit_response(msg), Some(82_700_000));
    }

    #[test]
    fn parse_cofl_profit_response_1b() {
        let msg = "According to our data TestPlayer made 1.5B in the last 30 days across 50 auctions";
        assert_eq!(parse_cofl_profit_response(msg), Some(1_500_000_000));
    }

    #[test]
    fn parse_cofl_profit_response_plain() {
        let msg = "According to our data TestPlayer made 500 in the last 1 days across 1 auctions";
        assert_eq!(parse_cofl_profit_response(msg), Some(500));
    }

    #[test]
    fn parse_cofl_profit_response_250k() {
        let msg = "According to our data TestPlayer made 250K in the last 7 days across 3 auctions";
        assert_eq!(parse_cofl_profit_response(msg), Some(250_000));
    }

    #[test]
    fn parse_cofl_profit_response_no_match() {
        assert_eq!(parse_cofl_profit_response("Some random chat message"), None);
    }

    #[test]
    fn parse_short_number_values() {
        assert_eq!(parse_short_number("82.7M"), Some(82_700_000));
        assert_eq!(parse_short_number("1.5B"), Some(1_500_000_000));
        assert_eq!(parse_short_number("250K"), Some(250_000));
        assert_eq!(parse_short_number("500"), Some(500));
        assert_eq!(parse_short_number("1,500,000"), Some(1_500_000));
        assert_eq!(parse_short_number("abc"), None);
    }

    #[test]
    fn parse_bz_list_flip_detail_profit() {
        let line = "128xWorm Membrane: 7.16M -> 7.91M => 741K(7)";
        let (name, profit, count) = parse_bz_list_flip_detail(line).unwrap();
        assert_eq!(name, "Worm Membrane");
        assert_eq!(profit, 741_000);
        assert_eq!(count, 7);
    }

    #[test]
    fn parse_bz_list_flip_detail_multiple_flips() {
        let line = "2xJungle Key: 1.05M -> 287K => -768K(1)";
        let (name, profit, count) = parse_bz_list_flip_detail(line).unwrap();
        assert_eq!(name, "Jungle Key");
        assert_eq!(profit, -768_000);
        assert_eq!(count, 1);
    }

    #[test]
    fn parse_bz_list_flip_detail_no_match() {
        assert!(parse_bz_list_flip_detail("Some random text").is_none());
        assert!(parse_bz_list_flip_detail("Last Completed Bazaar Flips").is_none());
    }

    #[test]
    fn parse_cofl_bz_h_negative_profit() {
        let msg = "Total Profit: -234M";
        assert_eq!(parse_cofl_bz_h_total_profit(msg), Some(-234_000_000));
    }

    #[test]
    fn parse_cofl_bz_h_positive_profit() {
        let msg = "Total Profit: 1.5B";
        assert_eq!(parse_cofl_bz_h_total_profit(msg), Some(1_500_000_000));
    }

    #[test]
    fn parse_cofl_bz_h_in_context() {
        let msg = "Bazaar Profit History for TestPlayer (last 7 days)\nTotal Profit: -234M\nAverage Daily Profit: -33.5M";
        assert_eq!(parse_cofl_bz_h_total_profit(msg), Some(-234_000_000));
    }

    #[test]
    fn parse_cofl_bz_h_no_match() {
        assert_eq!(parse_cofl_bz_h_total_profit("Some random message"), None);
    }
}
