package org.linuxlink.app

import android.content.Context

object AppPrefs {
    private const val PREFS = "app_prefs"
    private const val KEY_SECOND_SCREEN = "second_screen"

    fun isTablet(context: Context): Boolean =
        context.resources.configuration.smallestScreenWidthDp >= 600

    fun secondScreenEnabled(context: Context): Boolean =
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .getBoolean(KEY_SECOND_SCREEN, isTablet(context))

    fun setSecondScreenEnabled(context: Context, on: Boolean) {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
            .putBoolean(KEY_SECOND_SCREEN, on)
            .apply()
    }
}
