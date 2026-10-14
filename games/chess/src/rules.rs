use chess_board::ChessBoard;
use chess_foundation::{piece::PieceType, ChessMove};
use move_generator::{
    move_generator::get_all_legal_moves_for_color, piece_conductor::PieceConductor,
};

pub struct Game {
    pub board: ChessBoard,
    pub conductor: PieceConductor,
    pub history: Vec<String>,
    pub notation: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct MoveDelta {
    pub from: usize,
    pub to: usize,
    pub captured: Option<usize>,
    pub rook: Option<(usize, usize)>,
    pub promotion: Option<PieceType>,
}

pub fn square_name(square: u16) -> String {
    format!(
        "{}{}",
        (b'a' + (square % 8) as u8) as char,
        (b'1' + (square / 8) as u8) as char
    )
}

pub fn move_uci(mv: ChessMove) -> String {
    let suffix = match mv.promotion_piece_type() {
        Some(PieceType::Queen) => "q",
        Some(PieceType::Rook) => "r",
        Some(PieceType::Bishop) => "b",
        Some(PieceType::Knight) => "n",
        _ => "",
    };
    format!(
        "{}{}{suffix}",
        square_name(mv.start_square()),
        square_name(mv.target_square())
    )
}

impl Game {
    pub fn new() -> Self {
        Self {
            board: ChessBoard::new(),
            conductor: PieceConductor::new(),
            history: Vec::new(),
            notation: Vec::new(),
        }
    }

    pub fn legal_moves(&mut self) -> Vec<ChessMove> {
        let mut moves = Vec::new();
        let white = self.board.is_white_active();
        get_all_legal_moves_for_color(
            &mut self.board,
            &self.conductor,
            white,
            &mut moves,
            &mut Vec::new(),
        );
        moves
    }

    pub fn in_check(&self) -> bool {
        self.conductor
            .is_king_in_check(&self.board, self.board.is_white_active())
    }

    pub fn apply(&mut self, uci: &str) -> Result<MoveDelta, String> {
        let legal = self.legal_moves();
        let mut mv = legal
            .iter()
            .copied()
            .find(|m| move_uci(*m) == uci)
            .ok_or_else(|| format!("Engine returned illegal move: {uci}"))?;
        let from = mv.start_square() as usize;
        let to = mv.target_square() as usize;
        let piece = self
            .board
            .get_piece_at_square(from as u16)
            .ok_or("Empty source square")?;
        let en_passant = mv.has_flag(ChessMove::EN_PASSANT_CAPTURE_FLAG);
        let captured = if en_passant {
            Some(if piece.is_white() { to - 8 } else { to + 8 })
        } else {
            self.board.get_piece_at_square(to as u16).map(|_| to)
        };
        let castle = mv.has_flag(ChessMove::CASTLE_FLAG);
        let rook = castle.then(|| {
            if to > from {
                (from + 3, from + 1)
            } else {
                (from - 4, from - 1)
            }
        });
        let promotion = mv.promotion_piece_type();
        let mut san = if castle {
            if to > from {
                "O-O".to_owned()
            } else {
                "O-O-O".to_owned()
            }
        } else {
            let kind = piece.piece_type();
            let letter = match kind {
                PieceType::King => "K",
                PieceType::Queen => "Q",
                PieceType::Rook => "R",
                PieceType::Bishop => "B",
                PieceType::Knight => "N",
                PieceType::Pawn | PieceType::None => "",
            };
            let mut disambiguation = String::new();
            if kind == PieceType::Pawn && captured.is_some() {
                disambiguation.push((b'a' + (from % 8) as u8) as char);
            } else if kind != PieceType::Pawn {
                let peers: Vec<_> = legal
                    .iter()
                    .filter(|other| {
                        other.target_square() as usize == to
                            && other.start_square() as usize != from
                            && self
                                .board
                                .get_piece_at_square(other.start_square())
                                .is_some_and(|p| p.piece_type() == kind)
                    })
                    .collect();
                if !peers.is_empty() {
                    if peers
                        .iter()
                        .all(|m| m.start_square() as usize % 8 != from % 8)
                    {
                        disambiguation.push((b'a' + (from % 8) as u8) as char);
                    } else if peers
                        .iter()
                        .all(|m| m.start_square() as usize / 8 != from / 8)
                    {
                        disambiguation.push((b'1' + (from / 8) as u8) as char);
                    } else {
                        disambiguation = square_name(from as u16);
                    }
                }
            }
            let promo = promotion
                .map(|p| {
                    format!(
                        "={}",
                        match p {
                            PieceType::Queen => 'Q',
                            PieceType::Rook => 'R',
                            PieceType::Bishop => 'B',
                            _ => 'N',
                        }
                    )
                })
                .unwrap_or_default();
            format!(
                "{letter}{disambiguation}{}{}{promo}",
                if captured.is_some() { "x" } else { "" },
                square_name(to as u16)
            )
        };
        if !self.board.make_move(&mut mv) {
            return Err(format!("Could not apply {uci}"));
        }
        if self.in_check() {
            san.push(if self.legal_moves().is_empty() {
                '#'
            } else {
                '+'
            });
        }
        self.history.push(uci.to_owned());
        self.notation.push(san);
        Ok(MoveDelta {
            from,
            to,
            captured,
            rook,
            promotion,
        })
    }

    pub fn result(&mut self) -> Option<String> {
        if self.legal_moves().is_empty() {
            return Some(if self.in_check() {
                format!(
                    "{} wins by checkmate",
                    if self.board.is_white_active() {
                        "Ebony"
                    } else {
                        "Ivory"
                    }
                )
            } else {
                "Draw · stalemate".to_owned()
            });
        }
        if self.board.is_repetition(3) {
            return Some("Draw · threefold repetition".to_owned());
        }
        if self.board.get_halfmove_clock() >= 100 {
            return Some("Draw · fifty-move rule".to_owned());
        }
        let minors: Vec<_> = (0..64)
            .filter_map(|s| {
                self.board
                    .get_piece_at_square(s)
                    .map(|p| (s, p.piece_type()))
            })
            .filter(|(_, p)| *p != PieceType::King)
            .collect();
        let dead = minors.is_empty()
            || (minors.len() == 1 && matches!(minors[0].1, PieceType::Bishop | PieceType::Knight))
            || (minors.iter().all(|(_, p)| *p == PieceType::Bishop)
                && minors
                    .iter()
                    .all(|(s, _)| (s / 8 + s % 8) % 2 == (minors[0].0 / 8 + minors[0].0 % 8) % 2));
        if dead {
            return Some("Draw · insufficient material".to_owned());
        }
        // A spectator session should always progress to another game.
        if self.history.len() >= 400 {
            return Some("Draw · exhibition move limit".to_owned());
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fen(value: &str) -> Game {
        let mut g = Game::new();
        g.board.set_from_fen(value);
        g
    }

    #[test]
    fn legal_validation_and_special_moves() {
        let mut g = Game::new();
        assert!(g.apply("e2e5").is_err());
        assert_eq!(g.history.len(), 0);
        for uci in ["e2e4", "a7a6", "e4e5", "d7d5"] {
            g.apply(uci).unwrap();
        }
        let delta = g.apply("e5d6").unwrap();
        assert_eq!(delta.captured, Some(35));
        assert!(g.board.get_piece_at_square(35).is_none());
        for (uci, rook) in [("e1g1", (7, 5)), ("e1c1", (0, 3))] {
            let mut g = fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1");
            assert_eq!(g.apply(uci).unwrap().rook, Some(rook));
            assert_eq!(g.board.get_piece_type(rook.1 as u16), Some(PieceType::Rook));
        }
        for (suffix, kind) in [
            ("q", PieceType::Queen),
            ("r", PieceType::Rook),
            ("b", PieceType::Bishop),
            ("n", PieceType::Knight),
        ] {
            let mut g = fen("7k/P7/8/8/8/8/8/7K w - - 0 1");
            assert_eq!(
                g.apply(&format!("a7a8{suffix}")).unwrap().promotion,
                Some(kind)
            );
            assert_eq!(g.board.get_piece_type(56), Some(kind));
        }
    }

    #[test]
    fn adjudicates_games_and_preserves_history() {
        let mut g = Game::new();
        for uci in ["f2f3", "e7e5", "g2g4", "d8h4"] {
            g.apply(uci).unwrap();
        }
        assert!(g.result().unwrap().contains("checkmate"));
        assert_eq!(g.notation.last().unwrap(), "Qh4#");
        assert!(fen("7k/5Q2/6K1/8/8/8/8/8 b - - 0 1")
            .result()
            .unwrap()
            .contains("stalemate"));
        assert!(fen("7k/8/8/8/8/8/8/K7 w - - 0 1")
            .result()
            .unwrap()
            .contains("material"));
        assert!(fen("7k/8/8/8/8/8/8/KR6 w - - 100 1")
            .result()
            .unwrap()
            .contains("fifty"));
        let mut g = Game::new();
        for _ in 0..2 {
            for uci in ["g1f3", "g8f6", "f3g1", "f6g8"] {
                g.apply(uci).unwrap();
            }
        }
        assert!(g.result().unwrap().contains("repetition"));
    }
}
