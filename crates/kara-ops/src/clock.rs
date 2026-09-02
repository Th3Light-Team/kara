//! El desfase horario local, que `kara-fs` tiene prohibido averiguar por su
//! cuenta.
//!
//! `DeletionDate` de un `.trashinfo` es **hora local sin sufijo de zona**
//! (FreeDesktop). `kara-fs` no lee la zona de la máquina a propósito: quiere ser
//! determinista y testeable, así que expone `TrashPolicy::utc_offset_seconds` y
//! espera que alguien se lo inyecte. Ese alguien es esta capa.
//!
//! Sin esto, `TrashPolicy::default()` estampa UTC: en esta máquina, cuatro horas
//! adelantadas. La papelera seguiría funcionando, pero la columna «Fecha de
//! eliminación» mentiría y ordenar por ella daría un orden equivocado respecto a
//! lo que escriben los demás exploradores.

use chrono::{Local, Offset};

/// Segundos al este de UTC **en este instante**.
///
/// Se consulta por operación y no una vez al arrancar: el desfase cambia con el
/// horario de verano, y una sesión larga que cruce el cambio empezaría a estampar
/// fechas con una hora de error.
#[must_use]
pub fn local_utc_offset_seconds() -> i32 {
    Local::now().offset().fix().local_minus_utc()
}

/// Un [`kara_fs::trash::TrashPolicy`] con el desfase local ya inyectado.
///
/// Existe para que nadie construya la política a mano y se deje el desfase a
/// cero sin darse cuenta: el defecto de `kara-fs` es UTC, correcto solo en
/// Greenwich, y un `.trashinfo` con la fecha corrida no da error en ningún
/// sitio — simplemente miente en la columna «Fecha de eliminación».
///
/// Esta es la vía por la que la capa de arriba debe pedir la política.
#[must_use]
pub fn trash_policy() -> kara_fs::trash::TrashPolicy {
    kara_fs::trash::TrashPolicy {
        utc_offset_seconds: local_utc_offset_seconds(),
        ..Default::default()
    }
}
