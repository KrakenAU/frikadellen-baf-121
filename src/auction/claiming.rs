use azalea::prelude::*;
use tracing::info;
use crate::types::BotState;
use crate::bot::client::{
    BotClientState,
    // utility fns (pub(crate) in client.rs)
    click_window_slot,
    send_raw_close,
    find_slot_by_name,
    get_item_display_name_from_slot, get_item_lore_from_slot,
    remove_mc_colors, is_my_auctions_window_title, is_claimable_auction_slot,
};


pub async fn handle_window_claiming_purchased(
    bot: &Client,
    state: &BotClientState,
    window_id: u8,
    window_title: &str,
) {
        if window_title.contains("Auction House") {
            // Hardcoded slot 13 for "Your Bids" navigation — matches TypeScript clickWindow(bot, 13)
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
            info!("[ClaimPurchased] Auction House opened - clicking slot 13 (Your Bids)");
            click_window_slot(bot, &state.last_window_id, window_id, 13).await;
        } else if window_title.contains("Your Bids") {
            info!("[ClaimPurchased] Your Bids opened - looking for Claim All or Sold item");
            // Wait for ContainerSetContent to arrive and populate slots
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
            let menu = bot.menu();
            let slots = menu.slots();
            let mut found = false;
            // First look for Claim All by name (most reliable, matches TypeScript pattern)
            if let Some(i) = find_slot_by_name(&slots, "Claim All") {
                info!("[ClaimPurchased] Found Claim All at slot {}", i);
                click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
                send_raw_close(bot, window_id, &state.handlers);
                *state.bot_state.write() = BotState::Idle;
                found = true;
            }
            if !found {
                // Look for purchased item with "Status: Sold!" in lore (TypeScript pattern)
                for (i, item) in slots.iter().enumerate() {
                    let lore = get_item_lore_from_slot(item);
                    let lore_lower = lore.join("\n").to_lowercase();
                    if lore_lower.contains("status:") && lore_lower.contains("sold") {
                        info!("[ClaimPurchased] Found purchased item with Sold status at slot {}", i);
                        click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
                        // Stay in ClaimingPurchased — next window should be BIN Auction View
                        found = true;
                        break;
                    }
                }
                if !found {
                    info!("[ClaimPurchased] Nothing to claim, closing window and going idle");
                    send_raw_close(bot, window_id, &state.handlers);
                    *state.bot_state.write() = BotState::Idle;
                }
            }
        } else if window_title.contains("BIN Auction View") || window_title.contains("Auction View") {
            info!("[ClaimPurchased] Auction View opened - clicking slot 31 to collect");
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            click_window_slot(bot, &state.last_window_id, window_id, 31).await;
            send_raw_close(bot, window_id, &state.handlers);
            *state.bot_state.write() = BotState::Idle;
        }
}

pub async fn handle_window_claiming_sold(
    bot: &Client,
    state: &BotClientState,
    window_id: u8,
    window_title: &str,
) {
        if window_title.contains("Auction House") {
            info!("[ClaimSold] Auction House opened - navigating to Manage Auctions (slot 15)");
            // Wait for ContainerSetContent to arrive and populate slots
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
            let menu = bot.menu();
            let slots = menu.slots();
            // Prefer name-based match so a Hypixel UI shift is handled automatically;
            // fall back to the well-known slot 15 (same fixed slot the Selling flow uses).
            if let Some(i) = find_slot_by_name(&slots, "Manage Auctions")
                .or_else(|| find_slot_by_name(&slots, "My Auctions"))
            {
                info!("[ClaimSold] Clicking Manage/My Auctions at slot {}", i);
                click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
            } else {
                info!("[ClaimSold] Manage/My Auctions not found by name, clicking slot 15");
                click_window_slot(bot, &state.last_window_id, window_id, 15).await;
            }
        } else if is_my_auctions_window_title(window_title) {
            info!("[ClaimSold] My/Manage Auctions opened - looking for claimable items");
            // Wait for ContainerSetContent to arrive and populate slots
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
            let menu = bot.menu();
            let slots = menu.slots();
            // Look for Claim All first
            if let Some(i) = find_slot_by_name(&slots, "Claim All") {
                info!("[ClaimSold] Clicking Claim All at slot {}", i);
                click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
                // Claim All finishes everything — close window and go idle
                send_raw_close(bot, window_id, &state.handlers);
                *state.bot_state.write() = BotState::Idle;
            } else {
                // Look for first claimable item
                let mut found = false;
                for (i, item) in slots.iter().enumerate() {
                    if is_claimable_auction_slot(item) {
                        info!("[ClaimSold] Clicking claimable item at slot {}", i);
                        click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
                        // Stay in ClaimingSold — Hypixel re-opens Manage Auctions after the detail
                        found = true;
                        break;
                    }
                }
                if !found {
                    info!("[ClaimSold] Nothing to claim, closing window and going idle");
                    send_raw_close(bot, window_id, &state.handlers);
                    *state.bot_state.write() = BotState::Idle;
                }
            }
        } else if window_title.contains("BIN Auction View") || window_title.contains("Auction View") {
            info!("[ClaimSold] Auction detail opened - looking for Claim button");
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            let menu = bot.menu();
            let slots = menu.slots();
            // Prefer fixed slot 31 in auction detail; use name matching only as fallback.
            let slot_31_name = slots.get(31).and_then(get_item_display_name_from_slot).unwrap_or_default();
            let slot_31_lower = remove_mc_colors(&slot_31_name).to_lowercase();
            if slot_31_lower.contains("claim") {
                info!("[ClaimSold] Clicking preferred Claim slot 31");
                click_window_slot(bot, &state.last_window_id, window_id, 31).await;
            } else if let Some(i) = find_slot_by_name(&slots, "Claim") {
                info!("[ClaimSold] Slot 31 not claimable, falling back to Claim name match at slot {}", i);
                click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
            } else {
                info!("[ClaimSold] Claim button not found, clicking slot 31 fallback");
                click_window_slot(bot, &state.last_window_id, window_id, 31).await;
            }
            // Spawn a short watchdog: if Hypixel doesn't re-open Manage Auctions within
            // 1.5 s, transition to Idle so the command queue can proceed.
            let claim_state_ref = state.bot_state.clone();
            tokio::spawn(async move {
                tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;
                if *claim_state_ref.read() == BotState::ClaimingSold {
                    info!("[ClaimSold] No follow-up window after 1.5s, going idle");
                    *claim_state_ref.write() = BotState::Idle;
                }
            });
        }
}

