//! Générateur pseudo-aléatoire du placement des nœuds (jitter d'apparition,
//! couronne initiale d'un snapshot).
//!
//! L'usage est purement cosmétique : SplitMix64 suffit, sans dépendance, et
//! une graine fixe rend les tests déterministes.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};

/// Générateur SplitMix64 (Steele, Lea, Flood 2014).
#[derive(Clone, Debug)]
pub struct Rng {
    /// État interne, avancé d'une constante de Weyl à chaque tirage.
    state: u64,
}

impl Rng {
    /// Générateur à graine fixe (tests, rendu reproductible).
    pub fn with_seed(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Générateur à graine imprévisible, tirée des clés aléatoires que la
    /// bibliothèque standard obtient du système pour `RandomState`.
    pub fn from_entropy() -> Self {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(0x5EED);
        Self::with_seed(hasher.finish())
    }

    /// Tirage brut sur 64 bits.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Flottant uniforme dans `[0, 1)`, au pas de 2⁻³² (un entier de 32 bits
    /// divisé par 2³²).
    pub fn random_float(&mut self) -> f64 {
        // Les 32 bits de poids fort : les meilleurs de SplitMix64.
        let high = (self.next_u64() >> 32) as u32;
        f64::from(high) / 4_294_967_296.0
    }
}

impl Default for Rng {
    fn default() -> Self {
        Self::from_entropy()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_float_reste_dans_l_intervalle_semi_ouvert() {
        let mut rng = Rng::with_seed(42);
        for _ in 0..10_000 {
            let v = rng.random_float();
            assert!((0.0..1.0).contains(&v), "{v}");
        }
    }

    #[test]
    fn meme_graine_meme_suite() {
        let mut a = Rng::with_seed(7);
        let mut b = Rng::with_seed(7);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }
}
