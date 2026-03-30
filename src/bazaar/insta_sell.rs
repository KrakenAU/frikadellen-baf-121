use azalea::prelude::*;
use std::sync::atomic::Ordering;
use tracing::{info, warn};
use crate::types::BotState;
use crate::bot::client::{
    BotClientState, InstaSellStep,
    // utility fns (pub(crate) in client.rs)
    click_window_slot, send_chat_command,
    send_raw_close,
    find_slot_by_name,
};


pub async fn handle_window_insta_selling(
    bot: &Client,
    state: &BotClientState,
    window_id: u8,
    window_title: &str,
) {
        // Sell a dominant inventory item via /bz → Sell Instantly to free space.
        // Triggered by ManageOrders when inventory is full and one item type occupies
        // more than half the player inventory slots.
        //
        // Flow (tracked by insta_sell_step):
        //   FindItem       — bazaar search page: find item by name, click it
        //   FindSellButton — item detail page: find "Sell Instantly", click it
        //   WaitConfirm    — confirmation/warning page: wait ≤5 s, confirm
        //
        // After confirmation the bot opens /bz and returns to ManagingOrders so the
        // collect loop can retry now that there is inventory space.
        let item_name = match state.bazaar.insta_sell_item.read().clone() {
            Some(name) => name,
            None => {
                warn!("[InstaSell] No item name stored, closing window and going idle");
                send_raw_close(bot, window_id, &state.handlers);
                *state.bot_state.write() = BotState::Idle;
                return;
            }
        };

        // Abort if this window has already been superseded
        if *state.last_window_id.read() != window_id {
            return;
        }

        let step = *state.bazaar.insta_sell_step.read();
        info!("[InstaSell] Window: \"{}\" | step: {:?} | item: \"{}\"", window_title, step, item_name);

        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
        if *state.last_window_id.read() != window_id { return; }

        if step == InstaSellStep::FindItem && window_title.contains("Bazaar") {
            // Search results: find the item by name and click it
            let poll_deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(1500);
            let item_slot = loop {
                if *state.last_window_id.read() != window_id { return; }
                let slots = bot.menu().slots();
                if let Some(i) = find_slot_by_name(&slots, &item_name) {
                    break Some(i);
                }
                if tokio::time::Instant::now() >= poll_deadline { break None; }
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            };
            match item_slot {
                Some(i) => {
                    if *state.last_window_id.read() != window_id { return; }
                    info!("[InstaSell] Found \"{}\" at slot {}, clicking", item_name, i);
                    *state.bazaar.insta_sell_step.write() = InstaSellStep::FindSellButton;
                    click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
                }
                None => {
                    warn!("[InstaSell] Item \"{}\" not found in bazaar search, closing window and going idle", item_name);
                    send_raw_close(bot, window_id, &state.handlers);
                    *state.bazaar.insta_sell_item.write() = None;
                    *state.bazaar.insta_sell_step.write() = InstaSellStep::FindItem;
                    *state.bot_state.write() = BotState::Idle;
                }
            }
        } else if step == InstaSellStep::FindSellButton {
            // Item detail page: find "Sell Instantly" and click it
            let poll_deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(1500);
            let sell_slot = loop {
                if *state.last_window_id.read() != window_id { return; }
                let slots = bot.menu().slots();
                if let Some(i) = find_slot_by_name(&slots, "Sell Instantly") {
                    break Some(i);
                }
                if tokio::time::Instant::now() >= poll_deadline { break None; }
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            };
            match sell_slot {
                Some(i) => {
                    if *state.last_window_id.read() != window_id { return; }
                    info!("[InstaSell] Clicking \"Sell Instantly\" at slot {}", i);
                    *state.bazaar.insta_sell_step.write() = InstaSellStep::WaitConfirm;
                    click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
                }
                None => {
                    warn!("[InstaSell] \"Sell Instantly\" not found, closing window and going idle");
                    send_raw_close(bot, window_id, &state.handlers);
                    *state.bazaar.insta_sell_item.write() = None;
                    *state.bazaar.insta_sell_step.write() = InstaSellStep::FindItem;
                    *state.bot_state.write() = BotState::Idle;
                }
            }
        } else if step == InstaSellStep::WaitConfirm {
            // Confirmation page (warning may be present for up to 5 seconds).
            // Wait up to 5 s for a "Confirm" button, then click it.
            info!("[InstaSell] Waiting up to 5s for confirm button...");
            let confirm_deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(5);
            let confirm_slot = loop {
                if *state.last_window_id.read() != window_id { return; }
                let slots = bot.menu().slots();
                if let Some(i) = find_slot_by_name(&slots, "Confirm") {
                    break Some(i);
                }
                if tokio::time::Instant::now() >= confirm_deadline { break None; }
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            };
            if *state.last_window_id.read() != window_id { return; }
            match confirm_slot {
                Some(i) => {
                    info!("[InstaSell] Clicking Confirm at slot {}", i);
                    click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
                }
                None => {
                    // Confirm button did not appear within 5 s — the sell may have already
                    // completed silently (no warning shown) or failed.  Log and continue so
                    // ManageOrders can retry; avoid clicking a random slot.
                    warn!("[InstaSell] Confirm button not found after 5s — sell may have completed or failed");
                }
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            // Reset and return to ManagingOrders so collect can retry
            info!("[InstaSell] Complete — returning to ManageOrders");
            *state.bazaar.insta_sell_item.write() = None;
            *state.bazaar.insta_sell_step.write() = InstaSellStep::FindItem;
            state.inventory_full.store(false, Ordering::Relaxed);
            *state.bot_state.write() = BotState::ManagingOrders;
            send_chat_command(bot, "/bz");
        }
}

