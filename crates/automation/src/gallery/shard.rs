//! One of several parts of the gallery, so that CI can render it on as many
//! runners at once (#286): on one runner it took 402 s of a 12-minute job.

use std::str::FromStr;

/// Part `number` of `count`: every `count`-th state, starting with the
/// `number`-th, in the gallery's order. The parts together hold every state
/// once, however many there are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shard {
    number: usize,
    count: usize,
}

impl Shard {
    /// Whether the state at `position` (from 0) in the gallery's order is
    /// in this part.
    pub fn holds(self, position: usize) -> bool {
        position % self.count == self.number - 1
    }
}

impl FromStr for Shard {
    type Err = String;

    /// `K/N`, with K from 1 to N.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let wrong = || format!("{text:?} is not K/N with K from 1 to N, such as 2/3");
        let (number, count) = text.split_once('/').ok_or_else(wrong)?;
        let number: usize = number.parse().map_err(|_| wrong())?;
        let count: usize = count.parse().map_err(|_| wrong())?;
        if number == 0 || number > count {
            return Err(wrong());
        }
        Ok(Self { number, count })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_parts_hold_every_state_once() {
        let parts: Vec<Shard> = ["1/3", "2/3", "3/3"]
            .iter()
            .map(|text| text.parse().expect("a shard"))
            .collect();
        for position in 0..10 {
            let holders = parts.iter().filter(|part| part.holds(position)).count();
            assert_eq!(holders, 1, "state {position}");
        }
        assert!(parts[1].holds(1) && parts[1].holds(4) && !parts[1].holds(2));
    }

    #[test]
    fn a_shard_outside_its_count_is_refused() {
        for text in ["0/3", "4/3", "3", "a/3", "1/0"] {
            assert!(text.parse::<Shard>().is_err(), "{text}");
        }
    }
}
