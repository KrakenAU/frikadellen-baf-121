use azalea::prelude::*;
use std::sync::atomic::Ordering;
use tracing::{info, warn};
use crate::types::BotState;
use crate::bot::client::{
    BotClientState,
    // utility fns (pub(crate) in client.rs)
    click_window_slot,
    send_raw_close,
    find_slot_by_name, get_item_lore_from_slot,
    clear_auction_preview_slot, click_window_slot_carrying
};
use crate::bot::steps::{AuctionStep};


pub async fn handle_window_selling(
    bot: &Client,
    state: &BotClientState,
    window_id: u8,
    window_title: &str,
) {
        // Full auction creation flow matching TypeScript sellHandler.ts
        // Exact slot numbers from TypeScript: slot 15 (AH nav), slot 48 (BIN type),
        // slot 31 (price setter), slot 33 (duration), slot 29 (confirm), slot 11 (final confirm)

        // Wait for ContainerSetContent to populate slots
        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

        let step = *state.auction.step.read();
        let item_name = state.auction.item_name.read().clone();
        let item_slot_opt = *state.auction.item_slot.read();
        let menu = bot.menu();
        let slots = menu.slots();

        info!("[Auction] Window: \"{}\" | step: {:?}", window_title, step);

        // If the sell was aborted (e.g. stuck item in auction slot), do not
        // proceed — close the window and bail.  The retry task spawned by the
        // chat handler will re-open /ah once the window is closed.
        if state.auction.sell_aborted.load(Ordering::Relaxed) {
            warn!("[Auction] Window opened but auction sell aborted — closing window {}", window_id);
            send_raw_close(bot, window_id, &state.handlers);
            return;
        }

        match step {
            AuctionStep::Initial => {
                // "Auction House" opened — click slot 15 (nav to Manage Auctions)
                if window_title.contains("Auction House") {
                    info!("[Auction] AH opened, clicking slot 15 (Manage Auctions nav)");
                    *state.auction.step.write() = AuctionStep::OpenManage;
                    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                    click_window_slot(bot, &state.last_window_id, window_id, 15).await;
                }
            }
            AuctionStep::OpenManage => {
                // "Manage Auctions" opened — find "Create Auction" button by name
                if window_title.contains("Manage Auctions") {
                    if let Some(i) = find_slot_by_name(&slots, "Create Auction") {
                        // Check if auction limit reached (TypeScript: check lore for "maximum number")
                        let lore = get_item_lore_from_slot(&slots[i]);
                        let lore_text = lore.join(" ").to_lowercase();
                        if lore_text.contains("maximum") || lore_text.contains("limit") {
                            warn!("[Auction] Maximum auction count reached, going idle");
                            state.auction.at_limit.store(true, Ordering::Relaxed);
                            send_raw_close(bot, window_id, &state.handlers);
                            *state.bot_state.write() = BotState::Idle;
                            return;
                        }
                        info!("[Auction] Clicking Create Auction at slot {}", i);
                        *state.auction.step.write() = AuctionStep::ClickCreate;
                        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                        click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
                    } else {
                        warn!("[Auction] Create Auction not found in Manage Auctions, going idle");
                        send_raw_close(bot, window_id, &state.handlers);
                        *state.bot_state.write() = BotState::Idle;
                    }
                } else if window_title.contains("Create Auction") && !window_title.contains("BIN") {
                    // Co-op AH or similar: jumped directly to "Create Auction" — click slot 48 (BIN)
                    info!("[Auction] Skipped Manage Auctions, in Create Auction — clicking slot 48 (BIN)");
                    *state.auction.step.write() = AuctionStep::SelectBIN;
                    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                    click_window_slot(bot, &state.last_window_id, window_id, 48).await;
                } else if window_title.contains("Create BIN Auction") {
                    // Co-op AH opened "Create BIN Auction" directly (skipping Manage Auctions).
                    // Run the SelectBIN logic inline.
                    info!("[Auction] Co-op AH: jumped straight to Create BIN Auction, handling as SelectBIN");
                    let player_start = *menu.player_slots_range().start();
                    let target_slot = if let Some(mj_slot) = item_slot_opt {
                        if mj_slot >= 9 && mj_slot <= 44 {
                            let offset = (mj_slot as usize) - 9;
                            let ws = player_start + offset;
                            if ws < slots.len() && !slots[ws].is_empty() {
                                Some(ws)
                            } else {
                                find_slot_by_name(&slots, &item_name)
                            }
                        } else {
                            find_slot_by_name(&slots, &item_name)
                        }
                    } else {
                        find_slot_by_name(&slots, &item_name)
                    };
                    if let Some(i) = target_slot {
                        info!("[Auction] Co-op AH: clicking item at slot {}", i);
                        let item_to_carry = slots[i].clone();
                        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                        click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
                        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
                        info!("[Auction] Co-op AH: clicking slot 31 (price setter)");
                        *state.auction.step.write() = AuctionStep::PriceSign;
                        click_window_slot_carrying(bot, &state.last_window_id, window_id, 31, &item_to_carry).await;
                    } else {
                        warn!("[Auction] Co-op AH: item \"{}\" not found, going idle", item_name);
                        send_raw_close(bot, window_id, &state.handlers);
                        *state.bot_state.write() = BotState::Idle;
                    }
                }
            }
            AuctionStep::ClickCreate => {
                // "Create Auction" opened — click slot 48 (BIN auction type)
                if window_title.contains("Create Auction") && !window_title.contains("BIN") {
                    info!("[Auction] Create Auction window opened, clicking slot 48 (BIN)");
                    *state.auction.step.write() = AuctionStep::SelectBIN;
                    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                    click_window_slot(bot, &state.last_window_id, window_id, 48).await;
                } else if window_title.contains("Create BIN Auction") {
                    // Hypixel sometimes opens "Create BIN Auction" directly after clicking
                    // "Create Auction" in Manage Auctions (skipping the type-select step).
                    // Run SelectBIN logic inline so the flow continues without getting stuck.
                    info!("[Auction] ClickCreate: jumped straight to Create BIN Auction, handling as SelectBIN");
                    // Clear a stuck item in the auction preview slot (same guard as SelectBIN).
                    clear_auction_preview_slot(bot, state, window_id, &slots).await;
                    let player_start = *menu.player_slots_range().start();
                    let target_slot = if let Some(mj_slot) = item_slot_opt {
                        if mj_slot >= 9 && mj_slot <= 44 {
                            let offset = (mj_slot as usize) - 9;
                            let ws = player_start + offset;
                            if ws < slots.len() && !slots[ws].is_empty() {
                                Some(ws)
                            } else {
                                find_slot_by_name(&slots, &item_name)
                            }
                        } else {
                            find_slot_by_name(&slots, &item_name)
                        }
                    } else {
                        find_slot_by_name(&slots, &item_name)
                    };
                    if let Some(i) = target_slot {
                        info!("[Auction] ClickCreate→SelectBIN: clicking item at slot {}", i);
                        let item_to_carry = slots[i].clone();
                        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                        click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
                        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
                        info!("[Auction] ClickCreate→SelectBIN: clicking slot 31 (price setter)");
                        *state.auction.step.write() = AuctionStep::PriceSign;
                        click_window_slot_carrying(bot, &state.last_window_id, window_id, 31, &item_to_carry).await;
                    } else {
                        warn!("[Auction] ClickCreate→SelectBIN: item \"{}\" not found, going idle", item_name);
                        send_raw_close(bot, window_id, &state.handlers);
                        *state.bot_state.write() = BotState::Idle;
                    }
                }
            }
            AuctionStep::SelectBIN => {
                // "Create BIN Auction" opened first time (setPrice=false in TS)
                // Find item by slot or by name, click it, then click slot 31 for price sign
                if window_title.contains("Create BIN Auction") {
                    // Proactive fix: if the auction item preview slot (slot 13) already
                    // has an item (e.g. from a previously failed auction creation or a
                    // purchased item auto-placed by the server), click it first to clear
                    // the slot so our item can be placed without triggering "You already
                    // have an item in the auction slot!".
                    clear_auction_preview_slot(bot, state, window_id, &slots).await;

                    // Calculate inventory slot: mineflayer_slot - 9 + window_player_start
                    let player_start = *menu.player_slots_range().start();
                    let target_slot = if let Some(mj_slot) = item_slot_opt {
                        // TypeScript: itemSlot = data.slot - bot.inventory.inventoryStart + sellWindow.inventoryStart
                        // mineflayer inventoryStart = 9; slots 9-44 are player inventory (36 slots)
                        if mj_slot >= 9 && mj_slot <= 44 {
                            let offset = (mj_slot as usize) - 9;
                            let ws = player_start + offset;
                            if ws < slots.len() && !slots[ws].is_empty() {
                                info!("[Auction] Using computed slot {} for item (mj_slot={})", ws, mj_slot);
                                Some(ws)
                            } else {
                                info!("[Auction] Computed slot {} empty/invalid, falling back to name search", ws);
                                find_slot_by_name(&slots, &item_name)
                            }
                        } else {
                            info!("[Auction] mj_slot {} out of expected range 9-44, falling back to name search", mj_slot);
                            find_slot_by_name(&slots, &item_name)
                        }
                    } else {
                        find_slot_by_name(&slots, &item_name)
                    };

                    if let Some(i) = target_slot {
                        info!("[Auction] Clicking item at slot {}", i);
                        let item_to_carry = slots[i].clone();
                        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                        click_window_slot(bot, &state.last_window_id, window_id, i as i16).await;
                        // Click slot 31 (price setter) — sign will open, handled in OpenSignEditor
                        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
                        info!("[Auction] Clicking slot 31 (price setter)");
                        *state.auction.step.write() = AuctionStep::PriceSign;
                        click_window_slot_carrying(bot, &state.last_window_id, window_id, 31, &item_to_carry).await;
                    } else {
                        warn!("[Auction] Item \"{}\" not found in Create BIN Auction window, going idle", item_name);
                        send_raw_close(bot, window_id, &state.handlers);
                        *state.bot_state.write() = BotState::Idle;
                    }
                }
            }
            AuctionStep::SetDuration => {
                // "Create BIN Auction" opened second time (setPrice=true, durationSet=false in TS)
                // Click slot 33 to open "Auction Duration" window
                if window_title.contains("Create BIN Auction") {
                    info!("[Auction] Price set, clicking slot 33 (duration)");
                    *state.auction.step.write() = AuctionStep::DurationSign;
                    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                    click_window_slot(bot, &state.last_window_id, window_id, 33).await;
                }
            }
            AuctionStep::DurationSign => {
                // "Auction Duration" window opened — click slot 16 to open sign for duration
                if window_title.contains("Auction Duration") {
                    info!("[Auction] Auction Duration window opened, clicking slot 16 (sign trigger)");
                    // Sign handler (OpenSignEditor) will fire and advance step to ConfirmSell
                    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                    click_window_slot(bot, &state.last_window_id, window_id, 16).await;
                }
            }
            AuctionStep::ConfirmSell => {
                // "Create BIN Auction" opened third time (setPrice=true, durationSet=true in TS)
                // Click slot 29 to proceed to "Confirm BIN Auction"
                if window_title.contains("Create BIN Auction") {
                    // Check if the sell was aborted (e.g. wrong item in auction slot)
                    if state.auction.sell_aborted.load(Ordering::Relaxed) {
                        warn!("[Auction] ConfirmSell aborted — wrong item detected, closing window");
                        send_raw_close(bot, window_id, &state.handlers);
                        // Stay in Selling state so the retry task can re-open /ah
                        return;
                    }
                    info!("[Auction] Both price and duration set, clicking slot 29 (confirm item)");
                    *state.auction.step.write() = AuctionStep::FinalConfirm;
                    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                    click_window_slot(bot, &state.last_window_id, window_id, 29).await;
                }
            }
            AuctionStep::FinalConfirm => {
                // "Confirm BIN Auction" window — click slot 11 to finalize.
                // AuctionListed event is emitted from the chat handler when Hypixel sends
                // "BIN Auction started for ..." (matches TypeScript sellHandler.ts).
                if window_title.contains("Confirm BIN Auction") || window_title.contains("Confirm") {
                    // Check if the sell was aborted (e.g. wrong item in auction slot)
                    if state.auction.sell_aborted.load(Ordering::Relaxed) {
                        warn!("[Auction] FinalConfirm aborted — wrong item detected, closing window");
                        send_raw_close(bot, window_id, &state.handlers);
                        // Stay in Selling state so the retry task can re-open /ah
                        return;
                    }
                    info!("[Auction] Confirm BIN Auction window, clicking slot 11 (final confirm)");
                    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                    click_window_slot(bot, &state.last_window_id, window_id, 11).await;
                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                    info!("[Auction] ===== AUCTION CREATED =====");
                    send_raw_close(bot, window_id, &state.handlers);
                    *state.bot_state.write() = BotState::Idle;
                }
            }
            // PriceSign step: no window interaction needed; sign handler does the work
            AuctionStep::PriceSign => {}
        }
}

