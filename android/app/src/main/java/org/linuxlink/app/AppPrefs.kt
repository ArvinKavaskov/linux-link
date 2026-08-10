package org.linuxlink.app

import android.content.Context

/**
 * The app's own few switches. Each default is chosen per device class, so the
 * app is set up correctly before anyone opens the settings: a tablet's whole
 * point here is to be a monitor, a phone's usually is not.
 */
object AppPrefs {

    private const val PREFS = "app_prefs"
    private const val KEY_SECOND_SCREEN = "second_screen"

    /** Google's own line: 600 dp of smallest width is where tablets start. */
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
