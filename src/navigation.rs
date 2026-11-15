/// Remote and keyboard navigation share this entry point.
pub fn next_selection(current: usize, direction: i32, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    (current as i64 + i64::from(direction)).rem_euclid(count as i64) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_wraps_at_both_ends() {
        assert_eq!(next_selection(0, -1, 3), 2);
        assert_eq!(next_selection(2, 1, 3), 0);
        assert_eq!(next_selection(0, 1, 3), 1);
    }

    #[test]
    fn empty_and_single_app_are_safe() {
        assert_eq!(next_selection(0, -1, 0), 0);
        assert_eq!(next_selection(0, 1, 1), 0);
    }
}
