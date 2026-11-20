//! Navigation au clavier dans une liste dont certains éléments sont
//! inactifs (options désactivées, séparateurs). Sans fenêtre, testée seule.

/// Élément actif suivant (`forward`) ou précédent à partir de `current`.
///
/// Sans élément courant, part du début (ou de la fin en arrière). Avec
/// `wrap`, la recherche reprend à l'autre bout ; sinon elle s'arrête sur
/// l'élément courant. Renvoie `None` si aucun élément n'est actif.
pub fn step(enabled: &[bool], current: Option<usize>, forward: bool, wrap: bool) -> Option<usize> {
    let len = enabled.len();
    if len == 0 || !enabled.iter().any(|e| *e) {
        return None;
    }
    let Some(current) = current.filter(|c| *c < len) else {
        return if forward {
            first(enabled)
        } else {
            last(enabled)
        };
    };
    let mut ix = current;
    for _ in 0..len {
        ix = if forward {
            if ix + 1 >= len {
                if !wrap {
                    return enabled[current]
                        .then_some(current)
                        .or_else(|| last(enabled));
                }
                0
            } else {
                ix + 1
            }
        } else if ix == 0 {
            if !wrap {
                return enabled[current]
                    .then_some(current)
                    .or_else(|| first(enabled));
            }
            len - 1
        } else {
            ix - 1
        };
        if enabled[ix] {
            return Some(ix);
        }
    }
    Some(current).filter(|c| enabled[*c])
}

/// Premier élément actif.
pub fn first(enabled: &[bool]) -> Option<usize> {
    enabled.iter().position(|e| *e)
}

/// Dernier élément actif.
pub fn last(enabled: &[bool]) -> Option<usize> {
    enabled.iter().rposition(|e| *e)
}

/// Saut d'une page (`page` éléments) vers l'avant ou l'arrière, ramené sur
/// l'élément actif le plus proche.
pub fn page(enabled: &[bool], current: Option<usize>, forward: bool, page: usize) -> Option<usize> {
    let len = enabled.len();
    let current = current?;
    let target = if forward {
        (current + page).min(len.saturating_sub(1))
    } else {
        current.saturating_sub(page)
    };
    if enabled.get(target).copied().unwrap_or(false) {
        return Some(target);
    }
    // Élément inactif : on cherche le plus proche dans le sens du saut,
    // puis dans l'autre.
    let ahead = if forward {
        (target..len).find(|ix| enabled[*ix])
    } else {
        (0..=target).rev().find(|ix| enabled[*ix])
    };
    ahead.or_else(|| step(enabled, Some(target), !forward, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    const E: [bool; 5] = [true, false, true, true, false];

    #[test]
    fn saute_les_inactifs() {
        assert_eq!(step(&E, Some(0), true, false), Some(2));
        assert_eq!(step(&E, Some(2), false, false), Some(0));
        assert_eq!(step(&E, Some(3), true, false), Some(3), "bloqué en fin");
        assert_eq!(step(&E, Some(3), true, true), Some(0), "reprend au début");
        assert_eq!(step(&E, Some(0), false, true), Some(3), "reprend à la fin");
    }

    #[test]
    fn sans_courant() {
        assert_eq!(step(&E, None, true, false), Some(0));
        assert_eq!(step(&E, None, false, false), Some(3));
        assert_eq!(step(&[false, false], None, true, true), None);
        assert_eq!(step(&[], None, true, true), None);
    }

    #[test]
    fn bornes() {
        assert_eq!(first(&E), Some(0));
        assert_eq!(last(&E), Some(3));
        assert_eq!(first(&[false]), None);
    }

    #[test]
    fn pages() {
        let enabled = [true; 30];
        assert_eq!(page(&enabled, Some(0), true, 10), Some(10));
        assert_eq!(page(&enabled, Some(25), true, 10), Some(29));
        assert_eq!(page(&enabled, Some(5), false, 10), Some(0));
        assert_eq!(page(&E, Some(0), true, 1), Some(2), "cible inactive");
        assert_eq!(page(&E, None, true, 1), None);
    }
}
