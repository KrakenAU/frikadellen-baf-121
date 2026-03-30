/// Pure string parsers for chat messages (no bot state dependencies).

#[cfg(test)]
use once_cell::sync::Lazy;

#[cfg(test)]
static SOLD_FOR_PRICE_RE: Lazy<regex::Regex> =
    Lazy::new(|| regex::Regex::new(r"(?i)sold\s*for[: ]+\s*([0-9,]+)\s*coins").expect("valid sold-for regex"));
#[cfg(test)]
static SOLD_BUYER_RE: Lazy<regex::Regex> =
    Lazy::new(|| regex::Regex::new(r"(?i)buyer[: ]+\s*([^\n]+)").expect("valid sold-buyer regex"));

/// Parse "You purchased <item> for <price> coins!" → (item_name, price)
pub(crate) fn parse_purchased_message(msg: &str) -> Option<(String, u64)> {
    // "You purchased <item> for <price> coins!"
    let after = msg.strip_prefix("You purchased ")?;
    let for_idx = after.rfind(" for ")?;
    let item_name = after[..for_idx].to_string();
    let rest = &after[for_idx + 5..];
    let coins_idx = rest.find(" coins")?;
    let price_str = rest[..coins_idx].replace(',', "");
    let price: u64 = price_str.trim().parse().ok()?;
    Some((item_name, price))
}



/// Parse "[Auction] <buyer> bought <item> for <price> coins" → (buyer, item_name, price)
pub(crate) fn parse_sold_message(msg: &str) -> Option<(String, String, u64)> {
    // "[Auction] <buyer> bought <item> for <price> coins"
    let after = msg.strip_prefix("[Auction] ")?;
    let bought_idx = after.find(" bought ")?;
    let buyer = after[..bought_idx].to_string();
    let rest = &after[bought_idx + 8..];
    let for_idx = rest.rfind(" for ")?;
    let item_name = rest[..for_idx].to_string();
    let rest2 = &rest[for_idx + 5..];
    let coins_idx = rest2.find(" coins")?;
    let price_str = rest2[..coins_idx].replace(',', "");
    let price: u64 = price_str.trim().parse().ok()?;
    Some((buyer, item_name, price))
}



/// Extract UUID from a message that might contain "/viewauction <UUID>".
/// Works in both plain-text context (UUID ends at whitespace) and JSON context
/// (UUID ends at `"` after the value string, e.g. from a serialized clickEvent).
/// Minecraft UUIDs consist only of hex digits and dashes, so `"` is never a valid
/// UUID character — using it as a terminator is unconditionally safe.
pub(crate) fn extract_viewauction_uuid(msg: &str) -> Option<String> {
    let idx = msg.find("/viewauction ")?;
    let rest = &msg[idx + 13..];
    let end = rest.find(|c: char| c.is_whitespace() || c == '"').unwrap_or(rest.len());
    let uuid = rest[..end].trim().to_string();
    if uuid.is_empty() { None } else { Some(uuid) }
}


#[cfg(test)]
pub(crate) fn parse_claimed_sold_event_from_lore(item_name: &str, lore: &[String]) -> Option<(String, u64, String)> {
    if lore.is_empty() {
        return None;
    }
    let combined = lore.join("\n");
    let combined_lower = combined.to_lowercase();
    let sold_status = (combined_lower.contains("status:") && combined_lower.contains("sold"))
        || combined_lower.contains("sold for");
    if !sold_status {
        return None;
    }

    let price_caps = SOLD_FOR_PRICE_RE.captures(&combined)?;
    let price_match = price_caps.get(1)?;
    let price: u64 = price_match.as_str().replace(',', "").trim().parse().ok()?;

    let buyer = SOLD_BUYER_RE
        .captures(&combined)
        .and_then(|caps| caps.get(1).map(|m| m.as_str().trim().to_string()))
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "Unknown".to_string());

    Some((item_name.to_string(), price, buyer))
}

