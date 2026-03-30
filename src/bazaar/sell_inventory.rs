use azalea::prelude::*;
use tracing::info;
use crate::types::BotState;
use crate::bot::client::{
    BotClientState, SellInventoryStep,
    // utility fns (pub(crate) in client.rs)
    click_window_slot,
    send_raw_close,
    find_slot_by_name, SELL_INVENTORY_NOW_FALLBACK_SLOT,
};


pub async fn handle_window_selling_inventory_bz(
    bot: &Client,
    state: &BotClientState,
    window_id: u8,
    window_title: &str,
) {
        // Sell whole inventory instantly via /bz → "Sell Inventory Now"
        // → "Selling whole inventory" (slot 11).
        //
        // Flow (tracked by sell_inventory_step):
        //   Initial       — bazaar main page: find & click "Sell Inventory Now"
        //   ConfirmWindow — confirmation page: click slot 11 to confirm
        //
        // If inventory has no instasellable items, Hypixel sends
        // "You don't have anything to sell!" in chat and does not open
        // the confirmation window.
        let step = *state.bazaar.sell_inventory_step.read();
        if *state.last_window_id.read() != window_id {
            return;
        }

        if step == SellInventoryStep::Initial && window_title.contains("Bazaar") {
            // Bazaar page (main or category) — find "Sell Inventory Now"
            // button dynamically.
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
            if *state.last_window_id.read() != window_id { return; }
            let slots = bot.menu().slots();
            let sell_inv_slot = find_slot_by_name(&slots, "Sell Inventory Now").unwrap_or(SELL_INVENTORY_NOW_FALLBACK_SLOT);
            info!("[SellInventoryBz] Bazaar window open — clicking 'Sell Inventory Now' at slot {}", sell_inv_slot);
            *state.bazaar.sell_inventory_step.write() = SellInventoryStep::ConfirmWindow;
            click_window_slot(bot, &state.last_window_id, window_id, sell_inv_slot as i16).await;
        } else if step == SellInventoryStep::ConfirmWindow {
            // Confirmation page — click slot 11 ("Selling whole inventory")
            info!("[SellInventoryBz] Confirmation window open — clicking slot 11 to sell");
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
            if *state.last_window_id.read() != window_id { return; }
            click_window_slot(bot, &state.last_window_id, window_id, 11).await;
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            info!("[SellInventoryBz] Done — closing window and going idle");
            send_raw_close(bot, window_id, &state.handlers);
            *state.bazaar.sell_inventory_step.write() = SellInventoryStep::Initial;
            *state.bot_state.write() = BotState::Idle;
        }
}

