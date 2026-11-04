//! 候选弹窗的不可变快照、布局命中、屏幕定位与陈旧动作校验。

use crate::engine::{Candidate, CompositionUpdate};
use thiserror::Error;

const BASE_DPI: u32 = 96;
const PADDING: i32 = 8;
const HEADER_HEIGHT: i32 = 30;
const CANDIDATE_HEIGHT: i32 = 30;
const FOOTER_HEIGHT: i32 = 22;
const FOOTER_BUTTON_WIDTH: i32 = 28;
const MIN_FOOTER_BUTTON_WIDTH: i32 = 12;
const MIN_CONTENT_WIDTH: i32 = 240;
const MAX_CONTENT_WIDTH: i32 = 640;
const MAX_VISIBLE_CANDIDATES: usize = 32;

/// 与平台无关的屏幕或客户区坐标点。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PopupPoint {
    /// 横坐标，可为负值。
    pub x: i32,
    /// 纵坐标，可为负值。
    pub y: i32,
}

/// 与平台无关的矩形。
///
/// 右下边界不包含在矩形内，空候选或失效 footer 使用 `None` 表示。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PopupRect {
    /// 左边界。
    pub left: i32,
    /// 上边界。
    pub top: i32,
    /// 右边界，不包含。
    pub right: i32,
    /// 下边界，不包含。
    pub bottom: i32,
}

impl PopupRect {
    /// 创建一个坐标矩形。
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    /// 返回非负宽度；倒置矩形按零处理。
    pub fn width(self) -> i32 {
        self.right.saturating_sub(self.left).max(0)
    }

    /// 返回非负高度；倒置矩形按零处理。
    pub fn height(self) -> i32 {
        self.bottom.saturating_sub(self.top).max(0)
    }

    /// 判断点是否位于矩形内，右边界和下边界不包含。
    pub const fn contains(self, point: PopupPoint) -> bool {
        point.x >= self.left && point.x < self.right && point.y >= self.top && point.y < self.bottom
    }

    /// 判断矩形是否具有正面积。
    pub const fn is_visible(self) -> bool {
        self.left < self.right && self.top < self.bottom
    }
}

/// 与平台无关的尺寸。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PopupSize {
    /// 宽。
    pub width: i32,
    /// 高。
    pub height: i32,
}

impl PopupSize {
    /// 创建一个尺寸。
    pub const fn new(width: i32, height: i32) -> Self {
        Self { width, height }
    }
}

/// 由同一份布局数据驱动的绘制区域和鼠标命中区域。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidatePopupLayout {
    /// 弹窗客户区尺寸，已限制在传入边界内。
    pub size: PopupSize,
    /// 顶部拼音预编辑区域。
    pub preedit: PopupRect,
    /// 当前页每个可见候选行的区域。
    pub candidates: Vec<PopupRect>,
    /// 页脚区域；无候选页时为 `None`。
    pub footer: Option<PopupRect>,
    /// “上一页”按钮区域；位于首页时为 `None`。
    pub previous_page: Option<PopupRect>,
    /// “下一页”按钮区域；位于末页时为 `None`。
    pub next_page: Option<PopupRect>,
    /// 页脚中间的页码区域。
    pub page_label: Option<PopupRect>,
}

impl CandidatePopupLayout {
    /// 根据快照、DPI 和可用最大尺寸计算布局。
    ///
    /// `bounds` 的左上角按客户区原点处理。若空间不足，会减少可见候选行并
    /// 隐藏放不下的 footer；窗口宽高不会超过边界。
    pub fn new(snapshot: &CandidatePopupSnapshot, dpi: u32, bounds: PopupSize) -> Self {
        let dpi = effective_dpi(dpi);
        let width_limit = bounds.width.max(1);
        let height_limit = bounds.height.max(1);
        let padding = scaled(PADDING, dpi);
        let header_height = scaled(HEADER_HEIGHT, dpi);
        let candidate_height = scaled(CANDIDATE_HEIGHT, dpi).max(1);
        let footer_height = scaled(FOOTER_HEIGHT, dpi);

        let mut widest = snapshot.preedit.chars().count().max(20);
        for candidate in &snapshot.candidates {
            let units = candidate.text.chars().count() + candidate.comment.chars().count() + 6;
            widest = widest.max(units);
        }
        let content_width = (widest.min(48) as i32)
            .saturating_mul(14)
            .clamp(MIN_CONTENT_WIDTH, MAX_CONTENT_WIDTH);
        let natural_width = scaled(content_width.saturating_add(PADDING * 2), dpi);
        let width = natural_width.min(width_limit).max(1);

        let desired_rows = snapshot.candidates.len().min(MAX_VISIBLE_CANDIDATES);
        let has_page_count = snapshot.page_count > 0;
        let fixed_height = padding.saturating_mul(2).saturating_add(header_height);
        let natural_height = fixed_height
            .saturating_add(candidate_height.saturating_mul(desired_rows as i32))
            .saturating_add(if has_page_count { footer_height } else { 0 });
        let height = natural_height.min(height_limit).max(1);
        let footer_visible = has_page_count && height >= fixed_height.saturating_add(footer_height);
        let row_budget = height
            .saturating_sub(fixed_height)
            .saturating_sub(if footer_visible { footer_height } else { 0 })
            .max(0);
        let visible_rows = desired_rows.min((row_budget / candidate_height) as usize);
        let footer_top = height.saturating_sub(padding).saturating_sub(footer_height);
        let body_bottom = if footer_visible {
            footer_top
        } else {
            height.saturating_sub(padding)
        };
        let candidates_top = padding.saturating_add(header_height);
        let candidates = (0..visible_rows)
            .map(|index| {
                let top =
                    candidates_top.saturating_add(candidate_height.saturating_mul(index as i32));
                PopupRect::new(
                    padding / 2,
                    top,
                    width.saturating_sub(padding / 2),
                    top.saturating_add(candidate_height).min(body_bottom),
                )
            })
            .collect();

        let footer = footer_visible.then(|| {
            PopupRect::new(
                padding,
                footer_top,
                width.saturating_sub(padding),
                height.saturating_sub(padding),
            )
        });
        let wants_previous = snapshot.page > 0;
        let wants_next = snapshot.page.saturating_add(1) < snapshot.page_count;
        let footer_inner_width = width.saturating_sub(padding.saturating_mul(2));
        let minimum_button_width = scaled(MIN_FOOTER_BUTTON_WIDTH, dpi).max(1);
        let footer_button_width = if wants_previous && wants_next {
            let available = footer_inner_width / 2;
            if available < minimum_button_width {
                0
            } else {
                scaled(FOOTER_BUTTON_WIDTH, dpi).min(available)
            }
        } else if wants_previous || wants_next {
            if footer_inner_width < minimum_button_width {
                0
            } else {
                scaled(FOOTER_BUTTON_WIDTH, dpi).min(footer_inner_width)
            }
        } else {
            0
        };
        let previous_page = footer.and_then(|footer| {
            (wants_previous && footer_button_width > 0).then(|| {
                PopupRect::new(
                    footer.left,
                    footer.top,
                    footer.left.saturating_add(footer_button_width),
                    footer.bottom,
                )
            })
        });
        let next_page = footer.and_then(|footer| {
            (wants_next && footer_button_width > 0).then(|| {
                PopupRect::new(
                    footer.right.saturating_sub(footer_button_width),
                    footer.top,
                    footer.right,
                    footer.bottom,
                )
            })
        });
        let label_left = previous_page.map_or(padding, |button| button.right);
        let label_right = next_page.map_or(width.saturating_sub(padding), |button| button.left);
        let page_label = footer.and_then(|footer| {
            (label_left < label_right)
                .then(|| PopupRect::new(label_left, footer.top, label_right, footer.bottom))
        });

        Self {
            size: PopupSize::new(width, height),
            preedit: PopupRect::new(
                padding,
                padding,
                width.saturating_sub(padding),
                padding.saturating_add(header_height),
            ),
            candidates,
            footer,
            previous_page,
            next_page,
            page_label,
        }
    }

    /// 将客户区坐标映射为携带快照 revision 的候选动作。
    ///
    /// header、页脚页码、窗口外和空候选区域都返回 `None`。首页的上一页和
    /// 末页的下一页没有命中区域，因此不会产生不可用动作。
    pub fn hit_test(
        &self,
        snapshot: &CandidatePopupSnapshot,
        point: PopupPoint,
    ) -> Option<CandidatePopupAction> {
        let window = PopupRect::new(0, 0, self.size.width, self.size.height);
        if !window.contains(point) {
            return None;
        }

        for (index, row) in self.candidates.iter().enumerate() {
            if row.contains(point) {
                return Some(CandidatePopupAction::Select {
                    revision: snapshot.revision,
                    page: snapshot.page,
                    index,
                });
            }
        }

        if self
            .previous_page
            .is_some_and(|button| button.contains(point))
        {
            return Some(CandidatePopupAction::PreviousPage {
                revision: snapshot.revision,
                page: snapshot.page,
            });
        }
        if self.next_page.is_some_and(|button| button.contains(point)) {
            return Some(CandidatePopupAction::NextPage {
                revision: snapshot.revision,
                page: snapshot.page,
            });
        }

        None
    }
}

/// 计算弹窗位置，优先放在插入符下方，放不下时翻到上方，再夹入工作区。
///
/// 尺寸会限制在工作区可见区域内。所有坐标计算使用 `i64` 中间值，支持负坐标
/// 显示器以及接近 `i32` 极值的输入。工作区没有正面积时返回 `None`。
pub fn position_candidate_popup(
    caret: PopupRect,
    desired_size: PopupSize,
    work_area: PopupRect,
) -> Option<PopupRect> {
    if !work_area.is_visible() {
        return None;
    }

    let work_left = i64::from(work_area.left);
    let work_top = i64::from(work_area.top);
    let work_right = i64::from(work_area.right);
    let work_bottom = i64::from(work_area.bottom);
    let work_width = work_right - work_left;
    let work_height = work_bottom - work_top;
    let width = i64::from(desired_size.width.max(1)).min(work_width);
    let height = i64::from(desired_size.height.max(1)).min(work_height);

    let below = i64::from(caret.bottom);
    let above = i64::from(caret.top).saturating_sub(height);
    let y = if below.saturating_add(height) <= work_bottom {
        below
    } else if above >= work_top {
        above
    } else {
        work_top
    };
    let x = i64::from(caret.left);
    let left = clamp_i64(x, work_left, work_right - width);
    let top = clamp_i64(y, work_top, work_bottom - height);

    Some(PopupRect::new(
        left as i32,
        top as i32,
        (left + width) as i32,
        (top + height) as i32,
    ))
}

/// 将已有矩形限制到工作区内，保留其左上角意图并夹紧尺寸。
///
/// `WM_DPICHANGED` 等已经给出建议位置时使用此函数；工作区没有正面积时返回
/// `None`。
pub fn clamp_candidate_popup(rect: PopupRect, work_area: PopupRect) -> Option<PopupRect> {
    if !work_area.is_visible() {
        return None;
    }

    let work_left = i64::from(work_area.left);
    let work_top = i64::from(work_area.top);
    let work_right = i64::from(work_area.right);
    let work_bottom = i64::from(work_area.bottom);
    let width = i64::from(rect.width().max(1)).min(work_right - work_left);
    let height = i64::from(rect.height().max(1)).min(work_bottom - work_top);
    let left = clamp_i64(
        i64::from(rect.left),
        work_left,
        work_right.saturating_sub(width),
    );
    let top = clamp_i64(
        i64::from(rect.top),
        work_top,
        work_bottom.saturating_sub(height),
    );

    Some(PopupRect::new(
        left as i32,
        top as i32,
        (left + width) as i32,
        (top + height) as i32,
    ))
}

/// Immutable state rendered by the candidate popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidatePopupSnapshot {
    /// Generation of the engine state represented by this snapshot.
    pub revision: u64,
    /// Raw Pinyin input associated with the candidates.
    pub preedit: String,
    /// Candidates on the current page only.
    pub candidates: Vec<Candidate>,
    /// Valid zero-based highlighted candidate on the current page.
    pub highlighted: Option<usize>,
    /// Zero-based current candidate page.
    pub page: usize,
    /// Number of candidate pages, or zero when there are no candidates.
    pub page_count: usize,
}

/// A popup interaction to validate against the latest snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidatePopupAction {
    /// Selects a candidate from the current page.
    Select {
        revision: u64,
        page: usize,
        index: usize,
    },
    /// Requests the previous candidate page.
    PreviousPage { revision: u64, page: usize },
    /// Requests the next candidate page.
    NextPage { revision: u64, page: usize },
}

/// An interaction that matched the latest popup snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptedCandidateAction {
    /// Selects a candidate from the validated page.
    Select { page: usize, index: usize },
    /// Requests the previous candidate page.
    PreviousPage { page: usize },
    /// Requests the next candidate page.
    NextPage { page: usize },
}

/// Reasons a popup interaction could not be accepted.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CandidatePopupError {
    /// No composition snapshot is currently displayed.
    #[error("the candidate popup has no visible snapshot")]
    NotVisible,

    /// The action belongs to an older or otherwise different snapshot.
    #[error("candidate popup action revision {actual} does not match revision {expected}")]
    StaleRevision { expected: u64, actual: u64 },

    /// The action targets a page other than the displayed page.
    #[error("candidate popup action page {actual} does not match page {expected}")]
    StalePage { expected: usize, actual: usize },

    /// The candidate index does not exist on the displayed page.
    #[error(
        "candidate index {index} is out of range for the {candidate_count} candidate(s) \
         on the current popup page"
    )]
    CandidateOutOfRange {
        index: usize,
        candidate_count: usize,
    },

    /// 首页没有可用的上一页动作。
    #[error("candidate popup page {page} has no previous page")]
    PreviousPageUnavailable { page: usize },

    /// 末页没有可用的下一页动作。
    #[error("candidate popup page {page} of {page_count} has no next page")]
    NextPageUnavailable { page: usize, page_count: usize },
}

/// Tracks the latest popup snapshot and validates interactions against it.
///
/// The model never dispatches actions to the engine. Callers validate one
/// action with [`CandidatePopupModel::accept_action`], then route the accepted
/// action through their engine session and apply the resulting update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidatePopupModel {
    snapshot: Option<CandidatePopupSnapshot>,
    next_revision: u64,
}

impl CandidatePopupModel {
    /// Creates a hidden model with its first snapshot revision ready.
    pub const fn new() -> Self {
        Self {
            snapshot: None,
            next_revision: 1,
        }
    }

    /// Returns the snapshot currently available for rendering.
    pub fn snapshot(&self) -> Option<&CandidatePopupSnapshot> {
        self.snapshot.as_ref()
    }

    /// Reports whether the popup should be visible.
    pub const fn is_visible(&self) -> bool {
        self.snapshot.is_some()
    }

    /// Applies an engine update to the popup state.
    ///
    /// Unhandled updates leave the current snapshot untouched. Commits,
    /// cancellations, and handled updates with an empty preedit hide the
    /// popup. Other handled updates replace the snapshot.
    pub fn apply_update(&mut self, update: &CompositionUpdate) {
        if !update.handled {
            return;
        }

        if update.committed_text().is_some() || update.cancelled || update.preedit.is_empty() {
            self.hide();
            return;
        }

        let revision = self.take_revision();
        let candidate_count = update.candidates.len();
        let highlighted = update.highlighted.filter(|index| *index < candidate_count);

        self.snapshot = Some(CandidatePopupSnapshot {
            revision,
            preedit: update.preedit.clone(),
            candidates: update.candidates.clone(),
            highlighted,
            page: update.page,
            page_count: update.page_count,
        });
    }

    /// Hides the popup and advances its revision.
    pub fn hide(&mut self) {
        self.take_revision();
        self.snapshot = None;
    }

    /// Validates a popup interaction without changing the snapshot.
    pub fn accept_action(
        &self,
        action: &CandidatePopupAction,
    ) -> Result<AcceptedCandidateAction, CandidatePopupError> {
        let snapshot = self
            .snapshot
            .as_ref()
            .ok_or(CandidatePopupError::NotVisible)?;

        match *action {
            CandidatePopupAction::Select {
                revision,
                page,
                index,
            } => {
                validate_revision(snapshot, revision)?;
                validate_page(snapshot, page)?;

                let candidate_count = snapshot.candidates.len();
                if index >= candidate_count {
                    return Err(CandidatePopupError::CandidateOutOfRange {
                        index,
                        candidate_count,
                    });
                }

                Ok(AcceptedCandidateAction::Select { page, index })
            }
            CandidatePopupAction::PreviousPage { revision, page } => {
                validate_revision(snapshot, revision)?;
                validate_page(snapshot, page)?;
                if page == 0 {
                    return Err(CandidatePopupError::PreviousPageUnavailable { page });
                }
                Ok(AcceptedCandidateAction::PreviousPage { page })
            }
            CandidatePopupAction::NextPage { revision, page } => {
                validate_revision(snapshot, revision)?;
                validate_page(snapshot, page)?;
                if page.saturating_add(1) >= snapshot.page_count {
                    return Err(CandidatePopupError::NextPageUnavailable {
                        page,
                        page_count: snapshot.page_count,
                    });
                }
                Ok(AcceptedCandidateAction::NextPage { page })
            }
        }
    }

    fn take_revision(&mut self) -> u64 {
        let revision = self.next_revision;
        self.next_revision = self.next_revision.wrapping_add(1);
        revision
    }
}

impl Default for CandidatePopupModel {
    fn default() -> Self {
        Self::new()
    }
}

fn validate_revision(
    snapshot: &CandidatePopupSnapshot,
    revision: u64,
) -> Result<(), CandidatePopupError> {
    if revision == snapshot.revision {
        Ok(())
    } else {
        Err(CandidatePopupError::StaleRevision {
            expected: snapshot.revision,
            actual: revision,
        })
    }
}

fn validate_page(
    snapshot: &CandidatePopupSnapshot,
    page: usize,
) -> Result<(), CandidatePopupError> {
    if page == snapshot.page {
        Ok(())
    } else {
        Err(CandidatePopupError::StalePage {
            expected: snapshot.page,
            actual: page,
        })
    }
}

fn effective_dpi(dpi: u32) -> u32 {
    if dpi == 0 {
        BASE_DPI
    } else {
        dpi.min(480)
    }
}

fn scaled(value: i32, dpi: u32) -> i32 {
    let scaled = (i64::from(value) * i64::from(effective_dpi(dpi)) + i64::from(BASE_DPI / 2))
        / i64::from(BASE_DPI);
    scaled.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

fn clamp_i64(value: i64, minimum: i64, maximum: i64) -> i64 {
    value.max(minimum).min(maximum.max(minimum))
}

/// Decision for publishing a completed edit back to its activation.
#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditPublishDecision {
    /// The activation and focus epoch still match; publish and show.
    PublishAndShow,
    /// The activation still matches but focus changed; publish while hidden.
    PublishWithoutPopup,
    /// The activation is gone or replaced; discard the stale result.
    Discard,
}

/// Decides whether a completed edit may be published to the active service.
///
/// A changed activation always discards the result. A changed focus epoch
/// keeps the document/engine state but prevents the popup from reappearing.
#[cfg(any(target_os = "windows", test))]
pub(crate) fn edit_publish_decision(
    current_activation: Option<u64>,
    expected_activation: u64,
    current_focus_epoch: u64,
    expected_focus_epoch: u64,
) -> EditPublishDecision {
    match current_activation {
        Some(current) if current == expected_activation => {
            if current_focus_epoch == expected_focus_epoch {
                EditPublishDecision::PublishAndShow
            } else {
                EditPublishDecision::PublishWithoutPopup
            }
        }
        _ => EditPublishDecision::Discard,
    }
}

/// 延迟鼠标动作在异步编辑入口前必须重新核对的票据。
#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DeferredActionTicket {
    /// 发起动作时的激活 token。
    pub activation: u64,
    /// 发起动作时的焦点 epoch。
    pub focus_epoch: u64,
    /// 动作所属 TSF context。
    pub context: usize,
    /// 动作对应的候选快照 revision。
    pub revision: u64,
}

/// 执行异步编辑前读取的当前激活/焦点状态。
#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DeferredActionState {
    /// 调用线程是否仍为该激活的所属线程。
    pub owner_thread_matches: bool,
    /// 当前激活 token。
    pub activation: Option<u64>,
    /// 当前是否仍拥有前台焦点。
    pub foreground: bool,
    /// 当前焦点 epoch。
    pub focus_epoch: u64,
    /// TSF 当前顶部 context 是否仍为动作目标。
    pub focus_context_matches: bool,
    /// 当前显示的 context。
    pub shown_context: Option<usize>,
    /// 当前候选快照 revision。
    pub revision: Option<u64>,
}

/// 判断延迟鼠标动作是否仍可进入宿主文档写路径。
#[cfg(any(target_os = "windows", test))]
pub(crate) fn deferred_action_is_valid(
    ticket: DeferredActionTicket,
    state: DeferredActionState,
) -> bool {
    state.owner_thread_matches
        && state.activation == Some(ticket.activation)
        && state.foreground
        && state.focus_epoch == ticket.focus_epoch
        && state.focus_context_matches
        && state.shown_context == Some(ticket.context)
        && state.revision == Some(ticket.revision)
}

#[cfg(test)]
mod tests {
    use super::{
        deferred_action_is_valid, edit_publish_decision, DeferredActionState, DeferredActionTicket,
        EditPublishDecision,
    };

    #[test]
    fn completed_edit_from_a_replaced_activation_is_discarded() {
        assert_eq!(
            edit_publish_decision(Some(2), 1, 0, 0),
            EditPublishDecision::Discard
        );
        assert_eq!(
            edit_publish_decision(None, 1, 0, 0),
            EditPublishDecision::Discard
        );
    }

    #[test]
    fn focus_loss_keeps_state_but_suppresses_the_popup() {
        assert_eq!(
            edit_publish_decision(Some(7), 7, 4, 3),
            EditPublishDecision::PublishWithoutPopup
        );
    }

    #[test]
    fn matching_activation_and_focus_allow_the_popup() {
        assert_eq!(
            edit_publish_decision(Some(7), 7, 3, 3),
            EditPublishDecision::PublishAndShow
        );
    }

    #[test]
    fn replaced_activation_wins_over_a_focus_change() {
        assert_eq!(
            edit_publish_decision(Some(8), 7, 4, 3),
            EditPublishDecision::Discard
        );
    }

    fn valid_ticket() -> DeferredActionTicket {
        DeferredActionTicket {
            activation: 7,
            focus_epoch: 3,
            context: 0x1234,
            revision: 11,
        }
    }

    fn valid_state() -> DeferredActionState {
        DeferredActionState {
            owner_thread_matches: true,
            activation: Some(7),
            foreground: true,
            focus_epoch: 3,
            focus_context_matches: true,
            shown_context: Some(0x1234),
            revision: Some(11),
        }
    }

    #[test]
    fn deferred_popup_action_requires_every_current_identity() {
        assert!(deferred_action_is_valid(valid_ticket(), valid_state()));

        let mut state = valid_state();
        state.activation = Some(8);
        assert!(!deferred_action_is_valid(valid_ticket(), state));

        let mut state = valid_state();
        state.foreground = false;
        assert!(!deferred_action_is_valid(valid_ticket(), state));

        let mut state = valid_state();
        state.focus_epoch += 1;
        assert!(!deferred_action_is_valid(valid_ticket(), state));

        let mut state = valid_state();
        state.shown_context = Some(0x5678);
        assert!(!deferred_action_is_valid(valid_ticket(), state));

        let mut state = valid_state();
        state.revision = Some(12);
        assert!(!deferred_action_is_valid(valid_ticket(), state));
    }

    #[test]
    fn deferred_popup_action_rejects_context_switch_even_with_same_revision() {
        let mut state = valid_state();
        state.focus_context_matches = false;
        state.revision = Some(valid_ticket().revision);
        assert!(!deferred_action_is_valid(valid_ticket(), state));
    }

    #[test]
    fn deferred_popup_action_rejects_wrong_thread() {
        let mut state = valid_state();
        state.owner_thread_matches = false;
        assert!(!deferred_action_is_valid(valid_ticket(), state));
    }
}
