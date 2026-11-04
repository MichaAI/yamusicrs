//! Ядро клиента Яндекс Музыки без UI: API, аудиодвижок, локальная библиотека,
//! офлайн-режим и Ynison. Интерфейс (TUI, Android) строится поверх.

pub mod api;
pub mod audio;
pub mod config;
pub mod fetch;
pub mod library;
pub mod lyrics;
pub mod offline;
pub mod offline_wave;
pub mod tinyml;
pub mod ynison;
