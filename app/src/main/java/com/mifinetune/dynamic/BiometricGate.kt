package com.mifinetune.dynamic

import android.content.Context
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity

/**
 * Optional biometric gate for sensitive actions (service off / restore).
 * Falls back to "allowed" when no biometric hardware is enrolled, so the
 * feature never blocks users on devices without a fingerprint/face setup.
 */
object BiometricGate {
    fun canAuthenticate(context: Context): Boolean {
        val bm = BiometricManager.from(context)
        return bm.canAuthenticate(BiometricManager.Authenticators.BIOMETRIC_WEAK) ==
            BiometricManager.BIOMETRIC_SUCCESS
    }

    fun authenticate(
        activity: FragmentActivity,
        title: String = "Confirm with biometrics",
        subtitle: String = "Protects service-off / restore",
        onSuccess: () -> Unit,
        onFailure: (String) -> Unit = {},
    ) {
        if (!canAuthenticate(activity)) {
            onSuccess()
            return
        }
        val executor = ContextCompat.getMainExecutor(activity)
        val prompt = BiometricPrompt(
            activity,
            executor,
            object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                    onSuccess()
                }
                override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                    onFailure(errString.toString())
                }
                override fun onAuthenticationFailed() {
                    // a single failed attempt; prompt stays up, no callback
                }
            },
        )
        val info = BiometricPrompt.PromptInfo.Builder()
            .setTitle(title)
            .setSubtitle(subtitle)
            .setNegativeButtonText("Cancel")
            .build()
        prompt.authenticate(info)
    }
}