package com.hkustgz.canvasai

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import android.util.Log
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Android Keystore–backed secret storage.
 *
 * The AES-256-GCM key is generated inside the `AndroidKeyStore` provider and is
 * never exported; only the IV + ciphertext are written to the app-private
 * SharedPreferences file. The Rust side calls the three static methods below via
 * JNI (see `src-tauri/src/security/secrets/android_keystore.rs`).
 *
 * Ciphertext format: base64( iv || ciphertext )
 */
object SecureStore {
    private const val TAG = "CaaSecureStore"
    private const val KEY_ALIAS = "caa_ai_secret_key"
    private const val PREFS = "caa_secure_store"
    private const val IV_BYTES = 12
    private const val TAG_BITS = 128
    private const val PROVIDER = "AndroidKeyStore"

    @Volatile
    private var appContext: Context? = null

    @JvmStatic
    fun init(context: Context) {
        appContext = context.applicationContext
    }

    private fun prefs(): android.content.SharedPreferences {
        val context = appContext ?: error("SecureStore.init(context) was not called")
        return context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
    }

    private fun secretKey(): SecretKey {
        val keyStore = KeyStore.getInstance(PROVIDER).apply { load(null) }
        val existing = keyStore.getEntry(KEY_ALIAS, null)
        if (existing is KeyStore.SecretKeyEntry) {
            return existing.secretKey
        }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, PROVIDER)
        generator.init(
            KeyGenParameterSpec.Builder(
                KEY_ALIAS,
                KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
            )
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build(),
        )
        return generator.generateKey()
    }

    @JvmStatic
    fun put(name: String, value: String): Boolean {
        return try {
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.ENCRYPT_MODE, secretKey())
            val ciphertext = cipher.doFinal(value.toByteArray(Charsets.UTF_8))
            val iv = cipher.iv
            val packed = ByteArray(iv.size + ciphertext.size)
            System.arraycopy(iv, 0, packed, 0, iv.size)
            System.arraycopy(ciphertext, 0, packed, iv.size, ciphertext.size)
            prefs().edit()
                .putString(name, Base64.encodeToString(packed, Base64.NO_WRAP))
                .commit()
        } catch (error: Throwable) {
            Log.e(TAG, "failed to store secret", error)
            false
        }
    }

    @JvmStatic
    fun get(name: String): String? {
        return try {
            val encoded = prefs().getString(name, null)
            if (encoded == null) {
                null
            } else {
                val packed = Base64.decode(encoded, Base64.NO_WRAP)
                if (packed.size <= IV_BYTES) {
                    null
                } else {
                    val iv = packed.copyOfRange(0, IV_BYTES)
                    val ciphertext = packed.copyOfRange(IV_BYTES, packed.size)
                    val cipher = Cipher.getInstance("AES/GCM/NoPadding")
                    cipher.init(Cipher.DECRYPT_MODE, secretKey(), GCMParameterSpec(TAG_BITS, iv))
                    String(cipher.doFinal(ciphertext), Charsets.UTF_8)
                }
            }
        } catch (error: Throwable) {
            // A key invalidation (lock-screen change, backup restore) makes old
            // ciphertext undecryptable; report absence so the user can re-enter it.
            Log.w(TAG, "failed to read secret", error)
            null
        }
    }

    @JvmStatic
    fun delete(name: String): Boolean {
        return try {
            prefs().edit().remove(name).commit()
        } catch (error: Throwable) {
            Log.e(TAG, "failed to delete secret", error)
            false
        }
    }
}
