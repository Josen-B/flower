use flower::engine::{Candidate, CompositionUpdate, EngineError, EngineRouter, Key, KeyEvent};
use flower::platform::{
    clamp_candidate_popup, position_candidate_popup, AcceptedCandidateAction, CandidatePopupAction,
    CandidatePopupError, CandidatePopupLayout, CandidatePopupModel, PopupPoint, PopupRect,
    PopupSize,
};

fn popup_update(
    preedit: &str,
    candidates: &[(&str, &str)],
    highlighted: Option<usize>,
    page: usize,
    page_count: usize,
) -> CompositionUpdate {
    CompositionUpdate {
        handled: true,
        preedit: preedit.to_owned(),
        candidates: candidates
            .iter()
            .map(|(text, comment)| Candidate::new(*text, *comment))
            .collect(),
        highlighted,
        page,
        page_count,
        ..CompositionUpdate::default()
    }
}

fn display_model() -> CandidatePopupModel {
    let mut model = CandidatePopupModel::new();
    model.apply_update(&popup_update(
        "shi",
        &[("是", "shi"), ("时", "shi")],
        Some(1),
        0,
        2,
    ));
    model
}

fn page_model(page: usize, page_count: usize) -> CandidatePopupModel {
    let mut model = CandidatePopupModel::new();
    model.apply_update(&popup_update(
        "shi",
        &[("是", "shi"), ("时", "shi")],
        Some(0),
        page,
        page_count,
    ));
    model
}

#[test]
fn snapshot_copies_the_current_page_and_is_an_immutable_clone() {
    let model = display_model();
    let snapshot = model.snapshot().expect("visible popup snapshot");

    assert_eq!(snapshot.preedit, "shi");
    assert_eq!(
        snapshot.candidates,
        vec![Candidate::new("是", "shi"), Candidate::new("时", "shi")]
    );
    assert_eq!(snapshot.highlighted, Some(1));
    assert_eq!(snapshot.page, 0);
    assert_eq!(snapshot.page_count, 2);

    let copied = snapshot.clone();
    assert_eq!(&copied, snapshot);
}

#[test]
fn unhandled_update_keeps_the_previous_snapshot() {
    let mut model = display_model();
    let previous = model.snapshot().cloned().expect("initial snapshot");
    let mut ignored = popup_update("", &[], None, 0, 0);
    ignored.handled = false;

    model.apply_update(&ignored);

    assert_eq!(model.snapshot(), Some(&previous));
}

fn assert_hides_with_revision_advance(hidden_update: CompositionUpdate) {
    let mut model = display_model();
    let previous_revision = model.snapshot().expect("initial snapshot").revision;

    model.apply_update(&hidden_update);
    assert!(!model.is_visible());
    assert_eq!(model.snapshot(), None);

    model.apply_update(&popup_update("n", &[("你", "ni")], Some(0), 0, 1));
    assert_eq!(
        model.snapshot().expect("new snapshot").revision,
        previous_revision.wrapping_add(2)
    );
}

#[test]
fn commit_cancel_and_empty_preedit_hide_the_popup() {
    let mut commit = popup_update("", &[], None, 0, 0);
    commit.commit = Some("是".to_owned());

    let mut cancel = popup_update("", &[], None, 0, 0);
    cancel.cancelled = true;

    for hidden_update in [commit, cancel, popup_update("", &[], None, 0, 0)] {
        assert_hides_with_revision_advance(hidden_update);
    }
}

#[test]
fn snapshot_discards_invalid_highlight_and_supports_empty_candidates() {
    let mut model = CandidatePopupModel::new();
    model.apply_update(&popup_update(
        "shi",
        &[("是", "shi"), ("时", "shi")],
        Some(2),
        0,
        1,
    ));
    let snapshot = model.snapshot().expect("visible popup snapshot");
    assert_eq!(snapshot.highlighted, None);

    model.apply_update(&popup_update("xyz", &[], Some(0), 0, 0));
    let snapshot = model.snapshot().expect("visible empty candidate popup");
    assert!(snapshot.candidates.is_empty());
    assert_eq!(snapshot.highlighted, None);
    assert_eq!(snapshot.page_count, 0);
}

#[test]
fn stale_revision_page_and_index_are_rejected() {
    let model = display_model();
    let snapshot = model.snapshot().expect("visible popup snapshot");
    let revision = snapshot.revision;
    let page = snapshot.page;
    let candidate_count = snapshot.candidates.len();

    assert_eq!(
        model.accept_action(&CandidatePopupAction::Select {
            revision: revision.wrapping_add(1),
            page,
            index: 0,
        }),
        Err(CandidatePopupError::StaleRevision {
            expected: revision,
            actual: revision.wrapping_add(1),
        })
    );
    assert_eq!(
        model.accept_action(&CandidatePopupAction::NextPage {
            revision,
            page: page + 1,
        }),
        Err(CandidatePopupError::StalePage {
            expected: page,
            actual: page + 1,
        })
    );
    assert_eq!(
        model.accept_action(&CandidatePopupAction::Select {
            revision,
            page,
            index: candidate_count,
        }),
        Err(CandidatePopupError::CandidateOutOfRange {
            index: candidate_count,
            candidate_count,
        })
    );
}

#[test]
fn hidden_popup_rejects_actions() {
    let mut model = display_model();
    let snapshot = model.snapshot().expect("visible popup snapshot");
    let action = CandidatePopupAction::PreviousPage {
        revision: snapshot.revision,
        page: snapshot.page,
    };
    model.hide();

    assert_eq!(
        model.accept_action(&action),
        Err(CandidatePopupError::NotVisible)
    );
}

#[test]
fn matching_selection_and_paging_actions_are_accepted_without_mutating() {
    let model = page_model(1, 3);
    let snapshot = model.snapshot().expect("visible popup snapshot");
    let revision = snapshot.revision;
    let page = snapshot.page;

    assert_eq!(
        model.accept_action(&CandidatePopupAction::Select {
            revision,
            page,
            index: 1,
        }),
        Ok(AcceptedCandidateAction::Select { page, index: 1 })
    );
    assert_eq!(
        model.accept_action(&CandidatePopupAction::PreviousPage { revision, page }),
        Ok(AcceptedCandidateAction::PreviousPage { page })
    );
    assert_eq!(
        model.accept_action(&CandidatePopupAction::NextPage { revision, page }),
        Ok(AcceptedCandidateAction::NextPage { page })
    );
    assert_eq!(model.snapshot(), Some(snapshot));
}

#[test]
fn layout_hit_test_covers_candidates_footer_and_rejects_non_actions() {
    let model = page_model(1, 3);
    let snapshot = model.snapshot().expect("visible popup snapshot");
    let layout = CandidatePopupLayout::new(snapshot, 96, PopupSize::new(800, 600));

    let header = PopupPoint {
        x: layout.preedit.left,
        y: layout.preedit.top,
    };
    assert_eq!(layout.hit_test(snapshot, header), None);
    assert_eq!(
        layout.hit_test(
            snapshot,
            PopupPoint {
                x: layout.size.width,
                y: layout.size.height,
            },
        ),
        None
    );

    let first_row = layout.candidates[0];
    assert_eq!(
        layout.hit_test(
            snapshot,
            PopupPoint {
                x: first_row.left,
                y: first_row.top,
            },
        ),
        Some(CandidatePopupAction::Select {
            revision: snapshot.revision,
            page: snapshot.page,
            index: 0,
        })
    );

    let previous = layout.previous_page.expect("middle page has previous");
    assert_eq!(
        layout.hit_test(
            snapshot,
            PopupPoint {
                x: previous.left,
                y: previous.top,
            },
        ),
        Some(CandidatePopupAction::PreviousPage {
            revision: snapshot.revision,
            page: snapshot.page,
        })
    );
    let next = layout.next_page.expect("middle page has next");
    assert_eq!(
        layout.hit_test(
            snapshot,
            PopupPoint {
                x: next.left,
                y: next.top,
            },
        ),
        Some(CandidatePopupAction::NextPage {
            revision: snapshot.revision,
            page: snapshot.page,
        })
    );

    let label = layout.page_label.expect("middle page has label");
    assert_eq!(
        layout.hit_test(
            snapshot,
            PopupPoint {
                x: label.left,
                y: label.top,
            },
        ),
        None
    );
}

#[test]
fn boundary_pages_hide_unavailable_footer_actions() {
    let first = page_model(0, 2);
    let first_snapshot = first.snapshot().expect("first page");
    let first_layout = CandidatePopupLayout::new(first_snapshot, 96, PopupSize::new(800, 600));
    assert_eq!(first_layout.previous_page, None);
    assert!(first_layout.next_page.is_some());
    let first_footer = first_layout.footer.expect("first page has footer");
    assert_eq!(
        first_layout.hit_test(
            first_snapshot,
            PopupPoint {
                x: first_footer.left,
                y: first_footer.top,
            },
        ),
        None
    );

    let last = page_model(1, 2);
    let last_snapshot = last.snapshot().expect("last page");
    let last_layout = CandidatePopupLayout::new(last_snapshot, 96, PopupSize::new(800, 600));
    assert!(last_layout.previous_page.is_some());
    assert_eq!(last_layout.next_page, None);

    assert_eq!(
        first.accept_action(&CandidatePopupAction::PreviousPage {
            revision: first_snapshot.revision,
            page: first_snapshot.page,
        }),
        Err(CandidatePopupError::PreviousPageUnavailable { page: 0 })
    );
    assert_eq!(
        last.accept_action(&CandidatePopupAction::NextPage {
            revision: last_snapshot.revision,
            page: last_snapshot.page,
        }),
        Err(CandidatePopupError::NextPageUnavailable {
            page: 1,
            page_count: 2,
        })
    );
}

#[test]
fn empty_candidates_have_no_rows_or_footer_hit_targets() {
    let mut model = CandidatePopupModel::new();
    model.apply_update(&popup_update("xyz", &[], None, 0, 0));
    let snapshot = model.snapshot().expect("empty candidate popup");
    let layout = CandidatePopupLayout::new(snapshot, 96, PopupSize::new(800, 600));

    assert!(layout.candidates.is_empty());
    assert_eq!(layout.footer, None);
    assert_eq!(layout.previous_page, None);
    assert_eq!(layout.next_page, None);
    assert_eq!(layout.hit_test(snapshot, PopupPoint { x: 20, y: 50 }), None);
}

#[test]
fn dpi_scales_layout_and_narrow_bounds_clip_visible_area() {
    let model = display_model();
    let snapshot = model.snapshot().expect("visible popup snapshot");
    let normal = CandidatePopupLayout::new(snapshot, 96, PopupSize::new(800, 600));
    let high_dpi = CandidatePopupLayout::new(snapshot, 192, PopupSize::new(800, 600));
    assert!(high_dpi.size.width > normal.size.width);
    assert!(high_dpi.size.height > normal.size.height);

    let narrow = CandidatePopupLayout::new(snapshot, 192, PopupSize::new(80, 90));
    assert_eq!(narrow.size, PopupSize::new(80, 90));
    assert!(narrow.candidates.is_empty());
    assert_eq!(narrow.footer, None);
}

#[test]
fn narrow_footer_buttons_do_not_overlap_and_both_remain_hittable() {
    let model = page_model(1, 3);
    let snapshot = model.snapshot().expect("visible popup snapshot");
    let layout = CandidatePopupLayout::new(snapshot, 96, PopupSize::new(70, 600));
    let previous = layout.previous_page.expect("previous button fits");
    let next = layout.next_page.expect("next button fits");

    assert!(previous.right <= next.left);
    assert_eq!(
        layout.hit_test(
            snapshot,
            PopupPoint {
                x: previous.left,
                y: previous.top,
            },
        ),
        Some(CandidatePopupAction::PreviousPage {
            revision: snapshot.revision,
            page: snapshot.page,
        })
    );
    assert_eq!(
        layout.hit_test(
            snapshot,
            PopupPoint {
                x: next.right - 1,
                y: next.top,
            },
        ),
        Some(CandidatePopupAction::NextPage {
            revision: snapshot.revision,
            page: snapshot.page,
        })
    );

    let tiny = CandidatePopupLayout::new(snapshot, 96, PopupSize::new(30, 600));
    assert_eq!(tiny.previous_page, None);
    assert_eq!(tiny.next_page, None);
}

#[test]
fn candidate_rows_without_footer_remain_fully_visible_and_hittable() {
    let mut model = CandidatePopupModel::new();
    model.apply_update(&popup_update(
        "xyz",
        &[("甲", "jia"), ("乙", "yi")],
        Some(1),
        0,
        0,
    ));
    let snapshot = model.snapshot().expect("candidate popup without pages");
    let layout = CandidatePopupLayout::new(snapshot, 96, PopupSize::new(800, 600));
    let last = *layout.candidates.last().expect("two candidate rows");

    assert_eq!(layout.footer, None);
    assert!(last.is_visible());
    assert_eq!(
        layout.hit_test(
            snapshot,
            PopupPoint {
                x: last.left,
                y: last.bottom - 1,
            },
        ),
        Some(CandidatePopupAction::Select {
            revision: snapshot.revision,
            page: snapshot.page,
            index: 1,
        })
    );
}

fn assert_all_cells_hittable(
    snapshot: &flower::platform::CandidatePopupSnapshot,
    layout: &CandidatePopupLayout,
) {
    assert_eq!(layout.preedit, PopupRect::new(0, 0, 0, 0));
    assert_eq!(layout.candidates.len(), snapshot.candidates.len());
    for (index, cell) in layout.candidates.iter().enumerate() {
        assert!(cell.is_visible());
        assert!(cell.left >= 0 && cell.top >= 0);
        assert!(cell.right <= layout.size.width && cell.bottom <= layout.size.height);
        for point in [
            PopupPoint {
                x: cell.left,
                y: cell.top,
            },
            PopupPoint {
                x: cell.right - 1,
                y: cell.bottom - 1,
            },
        ] {
            assert_eq!(
                layout.hit_test(snapshot, point),
                Some(CandidatePopupAction::Select {
                    revision: snapshot.revision,
                    page: snapshot.page,
                    index,
                })
            );
        }
        for other in &layout.candidates[..index] {
            assert!(
                cell.right <= other.left
                    || other.right <= cell.left
                    || cell.bottom <= other.top
                    || other.bottom <= cell.top
            );
        }
        if let Some(footer) = layout.footer {
            assert!(cell.bottom <= footer.top);
        }
    }
}

#[test]
fn horizontal_single_row_preserves_even_legacy_oversized_pages() {
    for count in 1..=99 {
        let candidates = vec![("词", "ci"); count];
        let mut model = CandidatePopupModel::new();
        model.apply_update(&popup_update("ci", &candidates, Some(count - 1), 1, 3));
        let snapshot = model.snapshot().expect("page snapshot");
        let layout = CandidatePopupLayout::new(snapshot, 96, PopupSize::new(800, 600));
        assert_all_cells_hittable(snapshot, &layout);
        for cells in layout.candidates.windows(2) {
            assert_eq!(cells[0].top, cells[1].top);
            assert_eq!(cells[0].bottom, cells[1].bottom);
            assert_eq!(cells[0].right, cells[1].left);
        }
        if count == 99 {
            assert!(layout.size.width <= 560);
            let doubled = CandidatePopupLayout::new(snapshot, 192, PopupSize::new(1600, 1200));
            assert_all_cells_hittable(snapshot, &doubled);
            assert_eq!(doubled.size.width, layout.size.width * 2);
            assert_eq!(doubled.size.height, layout.size.height * 2);
            assert!(doubled
                .candidates
                .iter()
                .all(|cell| cell.top == doubled.candidates[0].top));
        }
    }
}

#[test]
fn shared_estimates_and_one_pixel_cells_preserve_the_whole_page() {
    let candidates = vec![("词", "ci"); 9];
    let mut model = CandidatePopupModel::new();
    model.apply_update(&popup_update("ci", &candidates, Some(8), 0, 1));
    let snapshot = model.snapshot().unwrap();
    // Nine 61-pixel cells require 549 pixels, exceeding the 544 content cap.
    // Verify exact estimates on a genuinely fitting page.
    let mut fitting = snapshot.clone();
    fitting.candidates.truncate(8);
    fitting.highlighted = Some(7);
    let normal = CandidatePopupLayout::new(&fitting, 96, PopupSize::new(800, 600));
    for (index, cell) in normal.candidates.iter().enumerate() {
        assert_eq!(
            cell.width() as u32,
            flower::engine::paging::candidate_cell_width(&snapshot.candidates[index].text, index)
        );
    }
    let capped = CandidatePopupLayout::new(snapshot, 96, PopupSize::new(800, 600));
    assert_all_cells_hittable(snapshot, &capped);
    assert_eq!(
        capped
            .candidates
            .iter()
            .map(|cell| cell.width())
            .sum::<i32>(),
        544
    );
    let tiny = CandidatePopupLayout::new(snapshot, 96, PopupSize::new(25, 600));
    assert_all_cells_hittable(snapshot, &tiny);
    assert!(tiny.candidates.iter().all(|cell| cell.width() == 1));
    let impossible = CandidatePopupLayout::new(snapshot, 96, PopupSize::new(24, 600));
    assert!(impossible.candidates.is_empty());
    assert_eq!(impossible.footer, None);
    assert_eq!(impossible.previous_page, None);
    assert_eq!(impossible.next_page, None);
}

#[test]
fn preedit_and_canonical_comments_do_not_affect_geometry_or_snapshot_data() {
    let model = display_model();
    let original = model.snapshot().unwrap();
    let mut changed = original.clone();
    changed.preedit = "raw'pinyin".repeat(100);
    for candidate in &mut changed.candidates {
        candidate.comment = "canonical'pronunciation".repeat(100);
    }
    let before = changed.clone();
    for dpi in [96, 144, 192] {
        let bounds = PopupSize::new(800, 600);
        let layout = CandidatePopupLayout::new(&changed, dpi, bounds);
        assert_eq!(layout, CandidatePopupLayout::new(original, dpi, bounds));
        assert_all_cells_hittable(&changed, &layout);
    }
    assert_eq!(changed, before);
}

#[test]
fn long_cells_shrink_in_one_row_without_losing_candidates_or_identity() {
    let mut model = CandidatePopupModel::new();
    model.apply_update(&popup_update(
        "long",
        &[("长词", ""), ("short", ""), ("很长的候选词组", "")],
        Some(2),
        1,
        3,
    ));
    let mut snapshot = model.snapshot().unwrap().clone();
    snapshot.candidates[0].text = "长".repeat(1000);
    let layout = CandidatePopupLayout::new(&snapshot, 96, PopupSize::new(340, 600));
    assert_all_cells_hittable(&snapshot, &layout);
    assert!(layout.candidates[0].width() <= 240);
    assert_eq!(layout.candidates[1].top, layout.candidates[0].top);
    assert_eq!(snapshot.candidates[0].text, "长".repeat(1000));

    for dpi in [96, 144, 192] {
        let narrow = CandidatePopupLayout::new(&snapshot, dpi, PopupSize::new(160, 600));
        assert_all_cells_hittable(&snapshot, &narrow);
        assert!(narrow.candidates.iter().all(|cell| cell.width() < 160));
    }
}

#[test]
fn insufficient_width_or_total_height_hides_the_whole_page_and_footer() {
    let model = page_model(1, 3);
    let snapshot = model.snapshot().unwrap();
    let normal = CandidatePopupLayout::new(snapshot, 96, PopupSize::new(800, 600));
    for bounds in [
        PopupSize::new(1, 1),
        PopupSize::new(17, 600),
        PopupSize::new(800, normal.size.height - 1),
    ] {
        let layout = CandidatePopupLayout::new(snapshot, 96, bounds);
        assert!(layout.size.width <= bounds.width && layout.size.height <= bounds.height);
        assert_eq!(layout.preedit, PopupRect::new(0, 0, 0, 0));
        assert!(layout.candidates.is_empty());
        assert_eq!(layout.footer, None);
        assert_eq!(layout.previous_page, None);
        assert_eq!(layout.next_page, None);
        assert_eq!(layout.page_label, None);
        for x in 0..layout.size.width {
            for y in 0..layout.size.height {
                assert_eq!(layout.hit_test(snapshot, PopupPoint { x, y }), None);
            }
        }
    }
    let exact = CandidatePopupLayout::new(snapshot, 96, normal.size);
    assert_all_cells_hittable(snapshot, &exact);
    assert_eq!(exact, normal);
    assert_eq!(
        model.accept_action(&CandidatePopupAction::Select {
            revision: snapshot.revision,
            page: snapshot.page,
            index: 1,
        }),
        Ok(AcceptedCandidateAction::Select {
            page: snapshot.page,
            index: 1
        })
    );
}

#[test]
fn placement_prefers_below_then_flips_above_and_clamps_to_work_area() {
    let work_area = PopupRect::new(0, 0, 800, 600);
    let desired = PopupSize::new(240, 180);
    let below = position_candidate_popup(PopupRect::new(100, 100, 120, 120), desired, work_area)
        .expect("caret with room below");
    assert_eq!(below, PopupRect::new(100, 120, 340, 300));

    let above = position_candidate_popup(PopupRect::new(700, 500, 720, 520), desired, work_area)
        .expect("caret with room above");
    assert_eq!(above, PopupRect::new(560, 320, 800, 500));

    let clamped = position_candidate_popup(
        PopupRect::new(-500, 10, -480, 20),
        PopupSize::new(1000, 2000),
        PopupRect::new(-100, -50, 200, 150),
    )
    .expect("oversized popup is clipped");
    assert_eq!(clamped, PopupRect::new(-100, -50, 200, 150));
}

#[test]
fn placement_supports_negative_and_extreme_coordinates_without_overflow() {
    let negative = position_candidate_popup(
        PopupRect::new(-1800, 300, -1780, 320),
        PopupSize::new(300, 200),
        PopupRect::new(-1920, 0, 0, 1080),
    )
    .expect("negative monitor");
    assert_eq!(negative, PopupRect::new(-1800, 320, -1500, 520));

    let extreme = position_candidate_popup(
        PopupRect::new(i32::MAX - 20, i32::MAX - 20, i32::MAX, i32::MAX),
        PopupSize::new(300, 200),
        PopupRect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX),
    )
    .expect("extreme i32 work area");
    assert!(extreme.left <= extreme.right);
    assert!(extreme.top <= extreme.bottom);
    assert_eq!(extreme.width(), 300);
    assert_eq!(extreme.height(), 200);

    assert_eq!(
        clamp_candidate_popup(
            PopupRect::new(900, -80, 1200, 100),
            PopupRect::new(0, 0, 800, 600),
        ),
        Some(PopupRect::new(500, 0, 800, 180))
    );
}

#[test]
fn action_rejects_stale_page_and_unavailable_boundary_pages() {
    let model = page_model(1, 2);
    let snapshot = model.snapshot().expect("visible popup snapshot");
    assert_eq!(
        model.accept_action(&CandidatePopupAction::Select {
            revision: snapshot.revision,
            page: 0,
            index: 0,
        }),
        Err(CandidatePopupError::StalePage {
            expected: 1,
            actual: 0,
        })
    );
    assert_eq!(
        model.accept_action(&CandidatePopupAction::NextPage {
            revision: snapshot.revision,
            page: snapshot.page,
        }),
        Err(CandidatePopupError::NextPageUnavailable {
            page: 1,
            page_count: 2,
        })
    );
}

fn router() -> EngineRouter {
    EngineRouter::new().expect("embedded MVP lexicon must load")
}

fn type_text(router: &mut EngineRouter, text: &str) -> CompositionUpdate {
    let mut update = CompositionUpdate::default();
    for character in text.chars() {
        update = router
            .process_key(KeyEvent::plain(Key::Char(character)))
            .expect("typing a letter cannot fail");
    }
    update
}

#[test]
fn router_paging_and_selection_return_composition_updates() {
    let mut router = router();
    let typed = type_text(&mut router, "shi");
    assert!(typed.page_count > 1);

    let second_page = router.next_page().expect("next page");
    assert_eq!(second_page.page, 1);
    assert_eq!(second_page.preedit, "shi");

    let first_page = router.previous_page().expect("previous page");
    assert_eq!(first_page.page, 0);
    let expected = first_page.candidates[1].text.clone();

    let selected = router.select_candidate(1).expect("candidate selection");
    assert_eq!(selected.commit.as_deref(), Some(expected.as_str()));
    assert!(!router.is_composing());
}

#[test]
fn router_selection_and_paging_propagate_engine_errors() {
    let mut router = router();
    assert_eq!(
        router.select_candidate(0),
        Err(EngineError::NoActiveComposition)
    );
    assert_eq!(router.next_page(), Err(EngineError::NoActiveComposition));
    assert_eq!(
        router.previous_page(),
        Err(EngineError::NoActiveComposition)
    );

    type_text(&mut router, "nihao");
    assert!(matches!(
        router.select_candidate(99),
        Err(EngineError::CandidateOutOfRange { .. })
    ));
    assert_eq!(router.preedit(), "nihao");
}
