use azalea::prelude::*;
use tracing::info;
use crate::types::BotState;
use crate::bot::client::{
    BotClientState, BotEvent,
    // utility fns (pub(crate) in client.rs)
    click_window_slot,
    send_raw_close,
    find_slot_by_name,
    get_item_display_name_from_slot, get_item_lore_from_slot,
    remove_mc_colors, is_my_auctions_window_title,
};
use crate::bot::client::extract_price_from_lore;


pub async fn handle_window_cancelling_auction(
    bot: &Client,
    state: &BotClientState,
    window_id: u8,
    window_title: &str,
) {
        // Cancel auction flow: /ah → Manage Auctions → find auction → Cancel Auction → Confirm
        if window_title.contains("Auction House") {
            info!("[CancelAuction] Auction House opened - navigating to Manage Auctions");
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
            let menu = bot.menu();
            let slots = menu.slots();
            if let Some(i) = find_slot_by_name(&slots, "Manage Auctions")
                .or_else(|| find_slot_by_name(&slots, "My Auctions"))
            {
                info!("[CancelAuction] Clicking Manage/My Auctions at slot {}", i);
                click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
            } else {
                info!("[CancelAuction] Manage/My Auctions not found by name, clicking slot 15");
                click_window_slot(bot, &state.last_window_id, window_id, 15).await;
            }
        } else if is_my_auctions_window_title(window_title) {
            info!("[CancelAuction] Manage Auctions opened - searching for target auction");
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
            let menu = bot.menu();
            let slots = menu.slots();
            let target_name = state.auction.cancel_item_name.read().clone();
            let target_bid = *state.auction.cancel_starting_bid.read();
            let target_lower = target_name.to_lowercase();
            // Find the auction slot matching item_name + starting_bid
            let mut found = false;
            for (i, item) in slots.iter().enumerate() {
                if item.is_empty() { continue; }
                let display_name = match get_item_display_name_from_slot(item) {
                    Some(n) => n,
                    None => continue,
                };
                let clean = remove_mc_colors(&display_name).to_lowercase();
                if !clean.contains(&target_lower) { continue; }
                // Verify price matches for accurate identification
                let lore = get_item_lore_from_slot(item);
                let price = extract_price_from_lore(&lore);
                if let Some(p) = price {
                    if p != target_bid { continue; }
                }
                // Verify this is an active auction (not sold/expired)
                let combined_lower = lore.join("\n").to_lowercase();
                if combined_lower.contains("sold!")
                    || combined_lower.contains("expired")
                    || combined_lower.contains("ended")
                {
                    continue;
                }
                info!("[CancelAuction] Found matching auction '{}' at slot {} (price: {:?})", display_name, i, price);
                click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
                found = true;
                break;
            }
            if !found {
                info!("[CancelAuction] Target auction not found in Manage Auctions, closing window and going idle");
                send_raw_close(bot, window_id, &state.handlers);
                *state.bot_state.write() = BotState::Idle;
            }
        } else if window_title.contains("BIN Auction View") || window_title.contains("Auction View") {
            info!("[CancelAuction] Auction detail opened - looking for Cancel Auction button");
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            let menu = bot.menu();
            let slots = menu.slots();
            if let Some(i) = find_slot_by_name(&slots, "Cancel Auction") {
                info!("[CancelAuction] Clicking Cancel Auction at slot {}", i);
                click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
            } else {
                info!("[CancelAuction] Cancel Auction button not found, closing window and going idle");
                send_raw_close(bot, window_id, &state.handlers);
                *state.bot_state.write() = BotState::Idle;
            }
            // Watchdog: go idle if no follow-up window in 2s
            let cancel_state_ref = state.bot_state.clone();
            tokio::spawn(async move {
                tokio::time::sleep(tokio::time::Duration::from_millis(2000)).await;
                if *cancel_state_ref.read() == BotState::CancellingAuction {
                    info!("[CancelAuction] No follow-up window after 2s, going idle");
                    *cancel_state_ref.write() = BotState::Idle;
                }
            });
        } else if window_title.contains("Confirm") {
            info!("[CancelAuction] Confirm window opened - confirming cancellation");
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            let menu = bot.menu();
            let slots = menu.slots();
            // Click Confirm button — typically slot 11 or find by name
            if let Some(i) = find_slot_by_name(&slots, "Confirm") {
                info!("[CancelAuction] Clicking Confirm at slot {}", i);
                click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
            } else {
                info!("[CancelAuction] Confirm not found by name, clicking slot 11");
                click_window_slot(bot, &state.last_window_id, window_id, 11).await;
            }
            info!("[CancelAuction] Auction cancellation confirmed, closing window and going idle");
            send_raw_close(bot, window_id, &state.handlers);
            let cancelled_name = state.auction.cancel_item_name.read().clone();
            let cancelled_bid = *state.auction.cancel_starting_bid.read();
            let _ = state.event_tx.send(BotEvent::AuctionCancelled {
                item_name: cancelled_name,
                starting_bid: cancelled_bid as u64,
            });
            *state.bot_state.write() = BotState::Idle;
        }
}

