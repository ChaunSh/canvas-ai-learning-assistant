//! Install the process-wide rustls crypto provider.
//!
//! We deliberately build rustls with the *ring* backend instead of the default
//! `aws-lc-rs`: aws-lc-sys needs CMake/NASM at build time, which is unavailable
//! here and awkward to provide for the Android NDK toolchain. rustls 0.23
//! requires an explicit provider when the `aws_lc_rs` feature is disabled.

use std::sync::Once;

static INIT: Once = Once::new();

pub fn install_crypto_provider() {
    INIT.call_once(|| {
        // Fails only if another provider was already installed, which is fine.
        if rustls::crypto::ring::default_provider()
            .install_default()
            .is_err()
        {
            tracing::debug!("a rustls crypto provider was already installed");
        } else {
            tracing::debug!("installed rustls ring crypto provider");
        }
    });
}
