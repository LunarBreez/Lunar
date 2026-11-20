//! Fin d'import : libération de l'état occupé avant le rafraîchissement.

/// Libère l'état occupé (overlay, verrou global) **avant** de rafraîchir
/// l'état « matrice remplie ». Ainsi, même si la vérification auprès du
/// moteur échoue après un import refusé, l'overlay et le verrou ne peuvent
/// pas rester actifs. Une erreur de rafraîchissement est remise à
/// `report_error`, jamais propagée.
pub async fn finalize_import_state<E, Fut>(
    release_busy: impl FnOnce(),
    refresh_has_data: impl FnOnce() -> Fut,
    report_error: impl FnOnce(E),
) where
    Fut: Future<Output = Result<(), E>>,
{
    release_busy();
    if let Err(error) = refresh_has_data().await {
        report_error(error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};

    /// Exécute un futur qui ne se suspend jamais (aucun exécuteur requis).
    fn run_ready<F: Future>(future: F) -> F::Output {
        let mut future = pin!(future);
        match future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        {
            Poll::Ready(output) => output,
            Poll::Pending => panic!("futur suspendu"),
        }
    }

    #[test]
    fn libere_toujours_l_ui_avant_un_refresh_en_erreur() {
        let busy = Cell::new(true);
        let calls = RefCell::new(Vec::<String>::new());
        let reported = RefCell::new(None);

        run_ready(finalize_import_state(
            || {
                busy.set(false);
                calls.borrow_mut().push("released".to_owned());
            },
            || {
                calls.borrow_mut().push(format!("refresh:{}", busy.get()));
                std::future::ready(Err("backend indisponible"))
            },
            |error| {
                *reported.borrow_mut() = Some(error);
                calls.borrow_mut().push(format!("reported:{}", busy.get()));
            },
        ));

        assert!(!busy.get());
        assert_eq!(*reported.borrow(), Some("backend indisponible"));
        assert_eq!(
            *calls.borrow(),
            ["released", "refresh:false", "reported:false"]
        );
    }

    #[test]
    fn rafraichit_aussi_apres_un_succes() {
        let busy = Cell::new(true);
        let refreshed = Cell::new(false);

        run_ready(finalize_import_state(
            || busy.set(false),
            || {
                refreshed.set(true);
                std::future::ready(Ok::<(), String>(()))
            },
            |error| panic!("aucune erreur attendue : {error}"),
        ));

        assert!(!busy.get());
        assert!(refreshed.get());
    }
}
