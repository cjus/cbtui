//! The rules of chess, borrowed from a library.
//!
//! cbtui never decides a move's legality itself: it uses `cb2026-chess`, the rules crate of
//! the cb2026 engine (legal moves, SAN, FEN, PGN, and when a game is over). This module is the
//! only place that names that crate, so another rules library could take its place here
//! without touching the rest of the program. The engine is a separate program reached over
//! the Chess Node Protocol ([`crate::node`]); linking its rules crate doesn't tie cbtui to it.

pub use cb2026_chess::game::{Game, Outcome};
pub use cb2026_chess::{Color, File, Move, MoveKind, Piece, PieceType, Position, Rank, Square};

/// The square on `file` (0 = a) and `rank` (0 = 1), or `None` off the board.
pub fn square(file: usize, rank: usize) -> Option<Square> {
    Some(Square::new(
        File::from_index(file)?,
        Rank::from_index(rank)?,
    ))
}

/// The legal moves from `from` in `position`.
pub fn moves_from(position: &Position, from: Square) -> Vec<Move> {
    position
        .legal_moves()
        .iter()
        .copied()
        .filter(|mv| mv.from() == from)
        .collect()
}

/// Where a move puts its piece, as a person sees it. The same as [`Move::to`], except for
/// castling: the rules library stores castling as the king taking its own rook (`e1h1`, the
/// form Chess960 needs), but the king lands on the g- or c-file (`e1g1`).
pub fn destination(mv: Move) -> Square {
    if mv.kind() != MoveKind::Castle {
        return mv.to();
    }
    let file = if mv.to().file() > mv.from().file() {
        File::G
    } else {
        File::C
    };
    Square::new(file, mv.from().rank())
}

/// The legal move from `from` to `to`, choosing a queen when a pawn promotes (a different
/// piece can be typed in SAN, `e8=N`). Castling is the king to its destination (`e1` to `g1`)
/// or onto its own rook (`e1` to `h1`).
pub fn move_between(position: &Position, from: Square, to: Square) -> Option<Move> {
    let candidates: Vec<Move> = moves_from(position, from)
        .into_iter()
        .filter(|mv| mv.to() == to || destination(*mv) == to)
        .collect();
    candidates
        .iter()
        .copied()
        .find(|mv| mv.promotion() == Some(PieceType::Queen))
        .or_else(|| candidates.first().copied())
}

/// A move in the coordinate notation CNP uses (`e2e4`, `e7e8q`).
pub fn to_uci(mv: Move) -> String {
    mv.to_uci(false)
}

/// The game's moves in coordinate notation, from its starting position.
pub fn game_moves_uci(game: &Game) -> Vec<String> {
    game.moves()
        .iter()
        .map(|played| to_uci(played.mv))
        .collect()
}

/// A line of moves in coordinate notation, played from `position`, as SAN. Stops at the first
/// move that isn't legal (a line from an engine that was searching another position).
pub fn line_to_san(position: &Position, line: &[String]) -> Vec<String> {
    let mut position = position.clone();
    let mut san = Vec::new();
    for text in line {
        let Some(mv) = position.parse_uci_move(text) else {
            break;
        };
        san.push(position.to_san(mv));
        position.make_move(mv);
    }
    san
}

/// A sentence for how a game ended, like `Checkmate: White wins (1-0)`.
pub fn describe_outcome(outcome: Outcome) -> String {
    let reason = match outcome {
        Outcome::Checkmate { .. } => "Checkmate",
        Outcome::Stalemate => "Stalemate",
        Outcome::Repetition => "Threefold repetition",
        Outcome::FiftyMoves => "Fifty-move rule",
        Outcome::InsufficientMaterial => "Insufficient material",
        Outcome::Resignation { .. } => "Resignation",
        Outcome::DrawAgreed => "Draw agreed",
        Outcome::Timeout { .. } => "Time out",
        Outcome::TimeoutDraw { .. } => "Time out against insufficient material",
        Outcome::Declared { .. } => "Result declared",
    };
    let verdict = match outcome.winner() {
        Some(Color::White) => "White wins",
        Some(Color::Black) => "Black wins",
        None => "draw",
    };
    format!("{reason}: {verdict} ({})", outcome.result())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn squares_count_from_a1() {
        assert_eq!(square(0, 0).unwrap().to_string(), "a1");
        assert_eq!(square(4, 3).unwrap().to_string(), "e4");
        assert_eq!(square(7, 7).unwrap().to_string(), "h8");
        assert!(square(8, 0).is_none());
    }

    #[test]
    fn a_click_promotes_to_a_queen() {
        let position = Position::from_fen("8/4P3/8/8/8/8/8/K6k w - - 0 1").unwrap();
        let mv = move_between(&position, square(4, 6).unwrap(), square(4, 7).unwrap()).unwrap();
        assert_eq!(to_uci(mv), "e7e8q");
    }

    #[test]
    fn castling_is_the_king_to_g1_or_c1_or_onto_its_rook() {
        let position = Position::from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").unwrap();
        let sq = |name: &str| name.parse::<Square>().unwrap();
        let san = |from, to| position.to_san(move_between(&position, sq(from), sq(to)).unwrap());
        assert_eq!(san("e1", "g1"), "O-O");
        assert_eq!(san("e1", "h1"), "O-O");
        assert_eq!(san("e1", "c1"), "O-O-O");
        assert_eq!(san("e1", "a1"), "O-O-O");
        let castle = move_between(&position, sq("e1"), sq("g1")).unwrap();
        assert_eq!(destination(castle), sq("g1"));
        assert_eq!(to_uci(castle), "e1g1");
        // A move that isn't castling lands where it says.
        let rook = move_between(&position, sq("h1"), sq("h5")).unwrap();
        assert_eq!(destination(rook), sq("h5"));
    }

    #[test]
    fn lines_become_san_until_one_is_illegal() {
        let line: Vec<String> = ["e2e4", "e7e5", "g1f3", "e2e4"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            line_to_san(&Position::startpos(), &line),
            ["e4", "e5", "Nf3"]
        );
    }

    #[test]
    fn outcomes_read_as_sentences() {
        assert_eq!(
            describe_outcome(Outcome::Checkmate {
                winner: Color::Black
            }),
            "Checkmate: Black wins (0-1)"
        );
        assert_eq!(
            describe_outcome(Outcome::Stalemate),
            "Stalemate: draw (1/2-1/2)"
        );
    }
}
