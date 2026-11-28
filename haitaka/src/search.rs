//! Alpha-beta mechanics shared by the board and completed-turn search adapters.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Bound {
    None = 0,
    Upper = 1,
    Lower = 2,
    Exact = 3,
}

impl Bound {
    pub const fn from_bits(bits: u8) -> Self {
        match bits & 0x3 {
            1 => Self::Upper,
            2 => Self::Lower,
            3 => Self::Exact,
            _ => Self::None,
        }
    }

    pub const fn for_score(score: i32, (alpha, beta): (i32, i32)) -> Self {
        if score >= beta {
            Self::Lower
        } else if score > alpha {
            Self::Exact
        } else {
            Self::Upper
        }
    }

    pub const fn can_cutoff(self, score: i32, (alpha, beta): (i32, i32)) -> bool {
        match self {
            Self::Exact => true,
            Self::Lower => score >= beta,
            Self::Upper => score <= alpha,
            Self::None => false,
        }
    }
}

/// The window and score as they stand before one ordered move is searched.
pub struct MoveContext {
    pub number: u32,
    pub window: (i32, i32),
    pub best_score: i32,
}

pub struct MoveUpdate<M> {
    pub best_move: Option<M>,
    pub best_score: i32,
    pub improved: bool,
    pub cutoff: bool,
    pub number: u32,
}

pub struct MoveSearchResult<M> {
    pub score: i32,
    pub best_move: Option<M>,
    pub may_store: bool,
}

/// Visit the ordered legal moves of one node. The adapter searches each child
/// with its history in place and observes progress only after a score settles.
/// A child returning `None` failed low in a reduced search.
pub fn search_ordered_moves<C, M: Copy, E>(
    context: &mut C,
    moves: impl IntoIterator<Item = M>,
    (original_alpha, beta): (i32, i32),
    initial_best_score: i32,
    mut search: impl FnMut(&mut C, M, MoveContext) -> Result<Option<i32>, E>,
    mut complete: impl FnMut(&mut C, M, MoveUpdate<M>) -> Result<(), E>,
) -> Result<MoveSearchResult<M>, E> {
    let mut alpha = original_alpha;
    let mut best_score = initial_best_score;
    let mut best_move = None;
    let mut reduced_fail_low = false;
    for (number, mv) in (1..).zip(moves) {
        let node = MoveContext {
            number,
            window: (alpha, beta),
            best_score,
        };
        let Some(score) = search(context, mv, node)? else {
            reduced_fail_low = true;
            continue;
        };
        let improved = score > best_score;
        if improved {
            best_score = score;
            best_move = Some(mv);
        }
        alpha = alpha.max(score);
        let cutoff = alpha >= beta;
        complete(
            context,
            mv,
            MoveUpdate {
                best_move,
                best_score,
                improved,
                cutoff,
                number,
            },
        )?;
        if cutoff {
            break;
        }
    }

    let may_store = !(reduced_fail_low && alpha == original_alpha);
    Ok(MoveSearchResult {
        score: if may_store {
            best_score
        } else {
            original_alpha
        },
        best_move,
        may_store,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildTrial {
    None,
    Pvs,
    Lmr,
}

impl ChildTrial {
    pub fn pvs(enabled: bool, depth: u8, (alpha, beta): (i32, i32), first: bool) -> Self {
        if enabled && !first && depth >= 2 && alpha + 1 < beta {
            Self::Pvs
        } else {
            Self::None
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ChildSearchPhase {
    Full,
    Trial,
    Research,
}

pub enum ChildResult {
    Score(i32),
    ReplayAfterCap,
}

impl ChildResult {
    fn score(self) -> i32 {
        match self {
            Self::Score(score) => score,
            Self::ReplayAfterCap => unreachable!("a full-window search cannot request a replay"),
        }
    }
}

/// Apply the trial policy to an already prepared child. The adapter receives
/// the child's window and handles evaluation state, budgets and interruptions.
/// An outermost trial interrupted by a soft cap can request a full replay.
pub fn search_child<E>(
    depth: u8,
    (alpha, beta): (i32, i32),
    trial: ChildTrial,
    is_mate_score: impl Fn(i32) -> bool,
    mut search: impl FnMut(u8, (i32, i32), ChildSearchPhase) -> Result<ChildResult, E>,
) -> Result<Option<i32>, E> {
    use ChildSearchPhase::{Full, Research, Trial};

    let (trial_depth, trial_beta) = match trial {
        ChildTrial::None => return Ok(Some(-search(depth, (-beta, -alpha), Full)?.score())),
        ChildTrial::Pvs => (depth, alpha + 1),
        ChildTrial::Lmr => (depth - 1, beta),
    };
    if let ChildResult::Score(value) = search(trial_depth, (-trial_beta, -alpha), Trial)? {
        let score = -value;
        match trial {
            ChildTrial::Pvs if score <= alpha || score >= beta => return Ok(Some(score)),
            ChildTrial::Lmr if score <= alpha && !is_mate_score(score) => return Ok(None),
            _ => {}
        }
    }
    Ok(Some(-search(depth, (-beta, -alpha), Research)?.score()))
}

/// Track the quiescence window and the best evasion independently of alpha.
pub struct QuiescenceWindow {
    alpha: i32,
    beta: i32,
    evasions_only: bool,
    best_evasion: Option<i32>,
}

pub struct QuiescenceUpdate {
    pub selected: bool,
    pub result: Option<i32>,
}

impl QuiescenceWindow {
    pub fn new(alpha: i32, beta: i32, evasions_only: bool) -> Self {
        Self {
            alpha,
            beta,
            evasions_only,
            best_evasion: None,
        }
    }

    pub fn alpha(&self) -> i32 {
        self.alpha
    }

    pub fn child_window(&self) -> (i32, i32) {
        (-self.beta, -self.alpha)
    }

    pub fn stand_pat(&mut self, score: i32) -> Option<i32> {
        if score >= self.beta {
            return Some(score);
        }
        self.alpha = self.alpha.max(score);
        None
    }

    pub fn record_child(&mut self, score: i32, capped: bool) -> QuiescenceUpdate {
        let improves_alpha = score > self.alpha;
        let improves_evasion =
            self.evasions_only && self.best_evasion.is_none_or(|best| score > best);
        if improves_evasion {
            self.best_evasion = Some(score);
        }
        self.alpha = self.alpha.max(score);
        let result = if capped {
            Some(self.best_evasion.unwrap_or(self.alpha))
        } else {
            (score >= self.beta).then_some(score)
        };
        QuiescenceUpdate {
            selected: if capped && self.evasions_only {
                improves_evasion
            } else {
                improves_alpha || improves_evasion || (!capped && result.is_some())
            },
            result,
        }
    }
}

/// Prefer a settled move when an iteration is interrupted. Callers choose
/// whether an incomplete iteration may supply a move before passing it here.
pub fn select_root_move<M>(
    partial: Option<M>,
    completed: Option<M>,
    fallback: impl FnOnce() -> Option<M>,
) -> (Option<M>, bool) {
    if let Some(mv) = partial.or(completed) {
        (Some(mv), true)
    } else {
        (fallback(), false)
    }
}
