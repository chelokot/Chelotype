#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SmoothScroll {
    pending_lines: i32,
}

impl SmoothScroll {
    pub fn enqueue(&mut self, lines: i32) {
        self.pending_lines += lines;
    }

    pub fn next_step(&mut self) -> Option<i32> {
        match self.pending_lines.cmp(&0) {
            std::cmp::Ordering::Greater => {
                self.pending_lines -= 1;
                Some(1)
            }
            std::cmp::Ordering::Less => {
                self.pending_lines += 1;
                Some(-1)
            }
            std::cmp::Ordering::Equal => None,
        }
    }

    pub fn pending_lines(&self) -> i32 {
        self.pending_lines
    }

    pub fn is_active(&self) -> bool {
        self.pending_lines != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drains_positive_scroll_one_line_per_frame() {
        let mut scroll = SmoothScroll::default();
        scroll.enqueue(3);

        assert_eq!(scroll.next_step(), Some(1));
        assert_eq!(scroll.next_step(), Some(1));
        assert_eq!(scroll.next_step(), Some(1));
        assert_eq!(scroll.next_step(), None);
    }

    #[test]
    fn coalesces_opposite_scroll_directions() {
        let mut scroll = SmoothScroll::default();
        scroll.enqueue(3);
        scroll.enqueue(-1);

        assert_eq!(scroll.pending_lines(), 2);
        assert_eq!(scroll.next_step(), Some(1));
        assert_eq!(scroll.next_step(), Some(1));
        assert_eq!(scroll.next_step(), None);
    }

    #[test]
    fn drains_negative_scroll_one_line_per_frame() {
        let mut scroll = SmoothScroll::default();
        scroll.enqueue(-2);

        assert_eq!(scroll.next_step(), Some(-1));
        assert_eq!(scroll.next_step(), Some(-1));
        assert!(!scroll.is_active());
    }
}
