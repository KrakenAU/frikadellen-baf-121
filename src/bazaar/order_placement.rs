use azalea::prelude::*;
use std::sync::atomic::Ordering;
use tracing::{info, debug, warn};
use crate::types::BotState;
use crate::bot::client::{
    BotClientState, BotEvent,
    // step enums
    // utility fns (pub(crate) in client.rs)
    click_window_slot,
    send_raw_close,
    find_slot_by_name,
    get_item_display_name_from_slot,
    log_bazaar_order_placed
};
use crate::bot::steps::{BazaarStep};


pub async fn handle_window_bazaar(
    bot: &Client,
    state: &BotClientState,
    window_id: u8,
    window_title: &str,
) {
        // Full bazaar order-placement flow matching TypeScript placeBazaarOrder().
        // Context (item_name, amount, price_per_unit, is_buy_order) was stored in
        // execute_command when the BazaarBuyOrder / BazaarSellOrder command ran.
        //
        // Steps:
        //  1. Search-results page  ("Bazaar" in title, step == Initial)
        //     → find the item by name, click it.
        //  2. Item-detail page  (has "Create Buy Order" / "Create Sell Offer" slot)
        //     → click the right button.
        //  3. Amount screen  (has "Custom Amount" slot, buy orders only)
        //     → click Custom Amount, then write sign.
        //  4. Price screen   (has "Custom Price" slot)
        //     → click Custom Price, then write sign.
        //  5. Confirm screen  (step == SetPrice, no other matching slot)
        //     → click slot 13.
        //
        // Sign writing is handled separately in the OpenSignEditor packet handler below.

        let item_name = state.bazaar.item_name.read().clone();
        let is_buy_order = *state.bazaar.is_buy_order.read();
        let current_step = *state.bazaar.step.read();

        info!("[Bazaar] Window: \"{}\" | step: {:?}", window_title, current_step);

        // Poll every 50ms for up to 1500ms for slots to be populated by ContainerSetContent.
        // Matching TypeScript's findAndClick() poll pattern (checks every 50ms, up to ~600ms).
        // This is more reliable than a fixed sleep because ContainerSetContent may arrive
        // at any time after OpenScreen.
        let poll_deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(1500);

        // Helper: read the current slots from the menu
        let read_slots = || {
            let menu = bot.menu();
            menu.slots()
        };

        // Determine which button name to look for on the item-detail page
        let order_btn_name = if is_buy_order { "Create Buy Order" } else { "Create Sell Offer" };

        // Step 2: Item-detail page — poll for the order-creation button.
        // Only relevant when we haven't clicked an order button yet (Initial or SearchResults).
        // Skipped for SelectOrderType and beyond because order buttons only appear on the
        // item-detail page, not on price/amount/confirm screens.
        if current_step == BazaarStep::Initial || current_step == BazaarStep::SearchResults {
            // Poll until we find either "Create Buy Order" or "Create Sell Offer"
            let order_button_slot = loop {
                // Guard: if a newer window has opened this handler is stale — bail out.
                if *state.last_window_id.read() != window_id {
                    debug!("[Bazaar] Window {} superseded during order-button poll, aborting", window_id);
                    return;
                }
                let slots = read_slots();
                let buy_s  = find_slot_by_name(&slots, "Create Buy Order");
                let sell_s = find_slot_by_name(&slots, "Create Sell Offer");
                let found = if is_buy_order { buy_s } else { sell_s };
                if found.is_some() {
                    break found;
                }
                // Also break early if we're on a search-results or amount/price screen
                // (those don't have order buttons, no point waiting)
                let has_custom_amount = find_slot_by_name(&slots, "Custom Amount").is_some();
                let has_custom_price  = find_slot_by_name(&slots, "Custom Price").is_some();
                if has_custom_amount || has_custom_price {
                    break None;
                }
                // Any window with "Bazaar" in the title is a search-results / category page —
                // those never contain order buttons, so break immediately.
                if window_title.contains("Bazaar") {
                    break None;
                }
                if tokio::time::Instant::now() >= poll_deadline {
                    // Log all non-empty slots for debugging
                    warn!("[Bazaar] Polling timed out waiting for \"{}\" in \"{}\"", order_btn_name, window_title);
                    for (i, item) in slots.iter().enumerate() {
                        if let Some(name) = get_item_display_name_from_slot(item) {
                            warn!("[Bazaar]   slot {}: {}", i, name);
                        }
                    }
                    break None;
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            };

            if let Some(i) = order_button_slot {
                // Final guard before clicking — reject if a newer window has taken over.
                if *state.last_window_id.read() != window_id {
                    debug!("[Bazaar] Window {} superseded before order-button click, aborting", window_id);
                    return;
                }
                info!("[Bazaar] Item detail: clicking \"{}\" at slot {}", order_btn_name, i);
                *state.bazaar.step.write() = BazaarStep::SelectOrderType;
                // Add randomized human-like delay before clicking (200-500ms)
                let jitter = 200 + (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().subsec_nanos() % 300) as u64;
                tokio::time::sleep(tokio::time::Duration::from_millis(jitter)).await;
                if *state.last_window_id.read() != window_id { return; }
                click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
                return;
            }
        }

        // Step 1: Search-results page — "Bazaar" in title, step == Initial.
        // Handles both "Bazaar" (plain search) and "Bazaar ➜ "ItemName"" (filtered results)
        // where the item appears in the grid but order buttons are not yet visible.
        if window_title.contains("Bazaar") && current_step == BazaarStep::Initial {
            info!("[Bazaar] Search results: looking for \"{}\"", item_name);
            *state.bazaar.step.write() = BazaarStep::SearchResults;

            // Poll briefly for the item to appear in search results
            let found = loop {
                if *state.last_window_id.read() != window_id { return; }
                let slots = read_slots();
                let f = find_slot_by_name(&slots, &item_name);
                if f.is_some() || tokio::time::Instant::now() >= poll_deadline {
                    break f;
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            };

            match found {
                Some(i) => {
                    if *state.last_window_id.read() != window_id { return; }
                    info!("[Bazaar] Found item at slot {}", i);
                    // Add randomized human-like delay (200-450ms)
                    let jitter = 200 + (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().subsec_nanos() % 250) as u64;
                    tokio::time::sleep(tokio::time::Duration::from_millis(jitter)).await;
                    if *state.last_window_id.read() != window_id { return; }
                    click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
                }
                None => {
                    warn!("[Bazaar] Item \"{}\" not found in search results; going idle", item_name);
                    send_raw_close(bot, window_id, &state.handlers);
                    *state.bot_state.write() = BotState::Idle;
                }
            }
            return;
        }

        // For steps 3-5: poll for the relevant button (Custom Amount / Custom Price) for up
        // to 1500ms matching the order-button poll above.  A single fixed sleep is unreliable
        // because ContainerSetContent may arrive at any time after OpenScreen.
        let poll_deadline2 = tokio::time::Instant::now() + tokio::time::Duration::from_millis(1500);
        let (amount_slot, price_slot) = loop {
            if *state.last_window_id.read() != window_id { return; }
            let slots = read_slots();
            let ca = if is_buy_order { find_slot_by_name(&slots, "Custom Amount") } else { None };
            let cp = find_slot_by_name(&slots, "Custom Price");
            if ca.is_some() || cp.is_some() || tokio::time::Instant::now() >= poll_deadline2 {
                break (ca, cp);
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        };

        // Step 3: Amount screen (buy orders only)
        if let (Some(i), true) = (amount_slot,
            is_buy_order && current_step == BazaarStep::SelectOrderType)
        {
            if *state.last_window_id.read() != window_id { return; }
            info!("[Bazaar] Amount screen: clicking Custom Amount at slot {}", i);
            *state.bazaar.step.write() = BazaarStep::SetAmount;
            click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
            // Sign response is sent in the OpenSignEditor packet handler
        }
        // Step 4: Price screen
        else if let (Some(i), true) = (price_slot,
            current_step == BazaarStep::SelectOrderType || current_step == BazaarStep::SetAmount)
        {
            if *state.last_window_id.read() != window_id { return; }
            info!("[Bazaar] Price screen: clicking Custom Price at slot {}", i);
            *state.bazaar.step.write() = BazaarStep::SetPrice;
            click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
            // Sign response is sent in the OpenSignEditor packet handler
        }
        // Step 5: Confirm screen — anything that opens after SetPrice
        else if current_step == BazaarStep::SetPrice {
            if *state.last_window_id.read() != window_id { return; }
            info!("[Bazaar] Confirm screen: clicking slot 13");
            *state.bazaar.step.write() = BazaarStep::Confirm;
            // Clear rejection flag before clicking so we only capture the
            // response to *this* placement attempt.
            state.bazaar.order_rejected.store(false, Ordering::Relaxed);
            // Add randomized human-like delay before confirming (300-700ms)
            let jitter = 300 + (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().subsec_nanos() % 400) as u64;
            tokio::time::sleep(tokio::time::Duration::from_millis(jitter)).await;
            click_window_slot(bot, &state.last_window_id, window_id, 13).await;

            // Wait briefly for the server to respond (limit/rejection message arrives asynchronously)
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

            if state.bazaar.at_limit.load(Ordering::Relaxed) {
                warn!("[Bazaar] Order rejected (at limit) — not emitting BazaarOrderPlaced");
            } else if state.bazaar.order_rejected.load(Ordering::Relaxed) {
                warn!("[Bazaar] Order rejected (price not competitive) — not emitting BazaarOrderPlaced");
            } else {
                let item = item_name.clone();
                let amount = *state.bazaar.amount.read();
                let price_per_unit = *state.bazaar.price_per_unit.read();
                let total_value = amount as f64 * price_per_unit;
                log_bazaar_order_placed(is_buy_order, &item, total_value);
                let _ = state.event_tx.send(BotEvent::BazaarOrderPlaced {
                    item_name: item,
                    amount,
                    price_per_unit,
                    is_buy_order,
                });
                info!("[Bazaar] ===== ORDER COMPLETE =====");
            }
            send_raw_close(bot, window_id, &state.handlers);
            *state.bot_state.write() = BotState::Idle;
        }
}

