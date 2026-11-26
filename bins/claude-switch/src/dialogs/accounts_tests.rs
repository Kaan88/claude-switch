use super::*;
use mdshared_text::Bitmap;
use mdshared_ui::gfx::Gfx;
use mdshared_ui::theme;

#[test]
fn refresh_and_usage_values_fit_inside_the_visible_accounts_body() {
    for width in [280.0, 640.0, 1000.0] {
        let card = dialog_card(640.0, 400.0, width, 600.0);
        let body = Rect::new(card.x + 1.0, card.y + 54.0, card.w - 2.0, 200.0);
        let (usage, refresh) = account_usage_layout(body, body.y + 15.0, 100.0);
        assert!(usage.x >= body.x);
        assert!(refresh.x >= usage.x && refresh.right() <= usage.right());
        assert!(body.right() - usage.right() >= DIALOG_PAD + 7.0);
        assert!(refresh.y >= body.y && refresh.bottom() <= body.bottom());
        assert_eq!(refresh.w, 86.0);
    }
}

#[test]
fn both_accounts_get_a_complete_refresh_target_when_scrolled_into_view_at_minimum_size() {
    let config = crate::demo::config();
    assert_eq!(config.accounts.len(), 2);
    let reading = claude_switch_core::usage::Reading {
        read_at: 0,
        limits: vec![claude_switch_core::usage::Limit::legacy("seven_day", 43.0, None)],
        breakdown: vec![
            claude_switch_core::usage::Breakdown { label: "Claude Code".into(), percent: Some(27.0) },
            claude_switch_core::usage::Breakdown { label: "Chats".into(), percent: None },
        ],
    };
    let usages: Vec<_> = (0..2)
        .map(|i| AccountUsage {
            plan: None,
            reading: (i == 0).then_some(&reading),
            attempt: None,
            signed_in: Some(i == 0),
            running: 0,
            refresh: true,
        })
        .collect();
    let (w, h) = (760.0, 480.0);
    let mut gfx = Gfx::new();
    for scale in [1.0, 1.25, 1.5] {
        let mut bitmap = Bitmap::new((w * scale) as u32, (h * scale) as u32);
        let mut painter = Painter::new(&mut bitmap, &mut gfx, scale);
        for (account, mut scroll) in [(0, 0.0), (1, f32::MAX)] {
            let mut hits = Vec::new();
            let mut draw =
                Draw { hits: &mut hits, hover: None, pressed: None, pal: theme::DARK, radius: 9.0, shadow: 0.0 };
            draw_accounts((&config, &Default::default(), &usages), &mut draw, &mut painter, w, h, &mut scroll, &[]);
            let id = ACCOUNT + account * 10 + ACT_REFRESH_USAGE;
            let (button, _) = hits.iter().find(|(_, target)| *target == Target::Dialog(id)).unwrap();
            let card = dialog_card(640.0, h, w, h);
            assert!(button.right() <= card.right() - DIALOG_PAD);
            assert!(button.y >= card.y + 54.0 && button.bottom() <= card.bottom() - DIALOG_FOOTER);
            assert_eq!((button.w, button.h), (86.0, 28.0));
        }
    }
}
