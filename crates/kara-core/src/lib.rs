//! Dominio puro de Kara: representación de entradas del sistema de ficheros,
//! ordenación, agrupación y filtrado.
//!
//! Esta capa es la base de la pila (`ui → ops → {fs, index} → core`) y **no hace
//! I/O ni conoce Qt**. Todo lo que vive aquí debe ser testeable sin tocar el disco.

#![forbid(unsafe_code)]
