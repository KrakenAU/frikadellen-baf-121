use azalea::prelude::*;
use tracing::{info, warn};
use crate::types::BotState;
use crate::bot::client::{
    BotClientState, BotEvent, CookieStep,
    // utility fns (pub(crate) in client.rs)
    click_window_slot, send_chat_command,
    send_raw_close,
    get_item_display_name_from_slot, get_item_lore_from_slot,
    parse_cookie_duration_secs,
};
use azalea_protocol::packets::game::s_set_carried_item::ServerboundSetCarriedItem;
use azalea_protocol::packets::game::s_use_item::ServerboundUseItem;
use azalea_protocol::packets::game::s_interact::InteractionHand;


pub async fn handle_window_checking_cookie(
    bot: &Client,
    state: &BotClientState,
    window_id: u8,
    _window_title: &str,
) {
        // Opened by /sbmenu — search every slot for a Booster Cookie buff indicator
        // (lore contains "Duration:"). Matches TypeScript cookieHandler.ts slot 51 check.
        info!("[Cookie] /sbmenu window opened — scanning for cookie buff...");
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        let menu = bot.menu();
        let slots = menu.slots();
        let auto_cookie_hours = *state.auto_cookie_hours.read();

        let mut cookie_time_secs: Option<u64> = None;
        for item in slots.iter() {
            let lore = get_item_lore_from_slot(item);
            let lore_text = lore.join(" ");
            if lore_text.to_lowercase().contains("duration") {
                let secs = parse_cookie_duration_secs(&lore_text);
                cookie_time_secs = Some(secs);
                break;
            }
        }

        // Close the SkyBlock menu before proceeding
        send_raw_close(bot, window_id, &state.handlers);
        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

        match cookie_time_secs {
            None => {
                // No duration found — either no cookie is active or menu didn't load
                info!("[Cookie] Cookie duration not found in /sbmenu — skipping buy");
                *state.bot_state.write() = BotState::Idle;
            }
            Some(secs) => {
                let hours = secs / 3600;
                let color = if hours >= auto_cookie_hours { "§a" } else { "§c" };
                let _ = state.event_tx.send(BotEvent::ChatMessage(format!(
                    "§f[§4BAF§f]: §3Cookie time remaining: {}{}h§3 (threshold: {}h)",
                    color, hours, auto_cookie_hours
                )));
                info!("[Cookie] Cookie time: {}h, threshold: {}h", hours, auto_cookie_hours);
                *state.cookie_time_secs.write() = secs;

                if hours >= auto_cookie_hours {
                    info!("[Cookie] Cookie time sufficient — skipping buy");
                    *state.bot_state.write() = BotState::Idle;
                } else {
                    // Need to buy a cookie — use get_purse() via scoreboard
                    let purse = state.get_purse().unwrap_or(0);
                    // Require at least 7.5M coins (1.5× 5M default price) before buying
                    const MIN_PURSE_FOR_COOKIE: u64 = 7_500_000;
                    if purse < MIN_PURSE_FOR_COOKIE {
                        let _ = state.event_tx.send(BotEvent::ChatMessage(format!(
                            "§f[§4BAF§f]: §c[AutoCookie] Not enough coins to buy cookie (need 7.5M, have {}M)",
                            purse / 1_000_000
                        )));
                        warn!("[Cookie] Insufficient coins ({}) — skipping cookie buy", purse);
                        *state.bot_state.write() = BotState::Idle;
                    } else {
                        info!("[Cookie] Buying cookie ({}h remaining < {}h threshold)...", hours, auto_cookie_hours);
                        let _ = state.event_tx.send(BotEvent::ChatMessage(
                            "§f[§4BAF§f]: §6[AutoCookie] Buying booster cookie...".to_string()
                        ));
                        *state.cookie_step.write() = CookieStep::Initial;
                        send_chat_command(bot, "/bz Booster Cookie");
                        *state.bot_state.write() = BotState::BuyingCookie;
                    }
                }
            }
        }
}

pub async fn handle_window_buying_cookie(
    bot: &Client,
    state: &BotClientState,
    window_id: u8,
    window_title: &str,
) {
        // Multi-step cookie buy flow matching TypeScript cookieHandler.ts buyCookie().
        let step = *state.cookie_step.read();
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        if step == CookieStep::Initial && window_title.contains("Bazaar") {
            // Bazaar search results: click slot 11 (the cookie item)
            info!("[Cookie] Bazaar opened — clicking cookie item (slot 11)");
            click_window_slot(bot, &state.last_window_id, window_id, 11).await;
            *state.cookie_step.write() = CookieStep::ItemDetail;
        } else if step == CookieStep::ItemDetail {
            // Cookie item detail: click slot 10 (Buy Instantly)
            info!("[Cookie] Cookie detail — clicking Buy Instantly (slot 10)");
            click_window_slot(bot, &state.last_window_id, window_id, 10).await;
            *state.cookie_step.write() = CookieStep::BuyConfirm;
        } else if step == CookieStep::BuyConfirm {
            // Atomically advance to WaitingForCookie before any sleeps.
            // This prevents concurrent window events (e.g. the Bazaar re-opening the
            // item-detail page after purchase) from triggering additional buys.
            // Lock is acquired, checked, updated, then released before any I/O.
            let claimed = {
                let mut step_write = state.cookie_step.write();
                if *step_write == CookieStep::BuyConfirm {
                    *step_write = CookieStep::WaitingForCookie;
                    true
                } else {
                    false
                }
            };
            if !claimed {
                // Another concurrent handler already processed this step — close
                // any stale window and bail out.
                info!("[Cookie] BuyConfirm already handled by another task — closing window");
                send_raw_close(bot, window_id, &state.handlers);
                return;
            }

            // Purchase confirmation: click slot 10 to confirm
            info!("[Cookie] Buy confirmation — clicking Confirm (slot 10)");
            click_window_slot(bot, &state.last_window_id, window_id, 10).await;
            // Let the purchase process; the Bazaar may re-open the item-detail window
            // after purchase — that is handled by the WaitingForCookie branch below.
            tokio::time::sleep(tokio::time::Duration::from_millis(2000)).await;
            // Close the purchase/item-detail window if still open
            send_raw_close(bot, window_id, &state.handlers);
            tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

            // Find cookie in inventory and consume it by right-clicking.
            // Matches TypeScript: bot.equip(item, 'hand') → bot.activateItem() → click slot 11.
            let menu = bot.menu();
            let all_slots = menu.slots();
            let player_range = menu.player_slots_range();
            let cookie_slot = all_slots[player_range.clone()].iter().enumerate().find_map(|(i, item)| {
                let name = get_item_display_name_from_slot(item).unwrap_or_default().to_lowercase();
                if name.contains("booster cookie") || name.contains("cookie") {
                    Some(i)
                } else {
                    None
                }
            });

            match cookie_slot {
                Some(idx) => {
                    info!("[Cookie] Found cookie at player inventory index {} — equipping and consuming", idx);
                    // Convert player-range-relative index to hotbar slot (0-8).
                    // Player slots: 0-26 = main inventory, 27-35 = hotbar (slots 36-44 in menu).
                    // If cookie is already in hotbar (idx >= 27), select that hotbar slot.
                    // Otherwise, move it to hotbar slot 0 first.
                    let hotbar_slot: u16 = if idx >= 27 {
                        // Already in hotbar — map to hotbar index 0-8
                        (idx - 27) as u16
                    } else {
                        // Cookie is in main inventory — need to swap it to hotbar.
                        // Open player inventory (container 0), click cookie slot, then hotbar slot 0.
                        let inv_slot = (*player_range.start() + idx) as i16;
                        // Pick up cookie
                        click_window_slot(bot, &state.last_window_id, 0, inv_slot).await;
                        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                        // Place in hotbar slot 0 (slot 36 in player inventory container)
                        click_window_slot(bot, &state.last_window_id, 0, 36).await;
                        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                        0
                    };

                    // Select the hotbar slot
                    bot.write_packet(ServerboundSetCarriedItem { slot: hotbar_slot });
                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

                    // Right-click to open the cookie GUI
                    bot.write_packet(ServerboundUseItem {
                        hand: InteractionHand::MainHand,
                        seq: 0,
                        y_rot: 0.0,
                        x_rot: 0.0,
                    });
                    info!("[Cookie] Right-clicked cookie — waiting for cookie GUI");

                    // Transition to ConsumingCookie so the next OpenScreen event
                    // (the cookie activation GUI) is handled correctly.
                    *state.cookie_step.write() = CookieStep::ConsumingCookie;
                }
                None => {
                    warn!("[Cookie] Cookie not found in inventory after purchase");
                    let _ = state.event_tx.send(BotEvent::ChatMessage(
                        "§f[§4BAF§f]: §c[AutoCookie] Cookie purchased but not found in inventory".to_string()
                    ));
                    *state.bot_state.write() = BotState::Idle;
                }
            }
        } else if step == CookieStep::WaitingForCookie {
            // Between clicking Confirm and right-clicking the cookie in inventory.
            // Any window that opens here (e.g. Bazaar re-opening item detail) is
            // unexpected — close it immediately and do nothing else.
            info!("[Cookie] Unexpected window while waiting for cookie in inventory — closing");
            send_raw_close(bot, window_id, &state.handlers);
        } else if step == CookieStep::ConsumingCookie {
            // Atomically claim the consume step so only one concurrent handler fires.
            // Lock is acquired, checked, updated, then released before any I/O.
            let claimed = {
                let mut step_write = state.cookie_step.write();
                if *step_write == CookieStep::ConsumingCookie {
                    // Advance past ConsumingCookie to prevent re-entry.
                    *step_write = CookieStep::Initial;
                    true
                } else {
                    false
                }
            };
            if !claimed {
                // Already handled — close stale window and bail.
                info!("[Cookie] ConsumingCookie already handled by another task — closing window");
                send_raw_close(bot, window_id, &state.handlers);
                return;
            }

            // Cookie GUI opened — click slot 11 to consume the cookie
            info!("[Cookie] Cookie GUI opened — clicking slot 11 to consume");
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            click_window_slot(bot, &state.last_window_id, window_id, 11).await;
            tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

            // Close the cookie GUI
            send_raw_close(bot, window_id, &state.handlers);

            let current_time = *state.cookie_time_secs.read();
            let new_hours = (current_time + 4 * 86400) / 3600;
            let old_hours = current_time / 3600;
            let _ = state.event_tx.send(BotEvent::ChatMessage(format!(
                "§f[§4BAF§f]: §aBought and consumed booster cookie! Time: {}h → {}h",
                old_hours, new_hours
            )));
            info!("[Cookie] Cookie consumed successfully! Time: {}h → {}h", old_hours, new_hours);
            *state.bot_state.write() = BotState::Idle;
        }
}

