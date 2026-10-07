package com.mifinetune

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.lifecycle.viewmodel.compose.viewModel
import com.mifinetune.ui.HomeScreen
import com.mifinetune.ui.HomeViewModel
import com.mifinetune.ui.theme.MiFineTuneTheme

/**
 * Single activity. All logic lives in [HomeViewModel] + the Rust core.
 */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        setContent {
            MiFineTuneTheme {
                val vm: HomeViewModel = viewModel()
                HomeScreen(vm)
            }
        }
    }
}
