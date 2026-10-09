package mifinetune;

/**
 * One-shot FPS switch helper (run via app_process as root).
 *
 * Calls MIUI's own display-feature API exactly like MiSettings'
 * RefreshRateActivity does (DisplayUtils.java): effect id 24 is the
 * refresh-rate screen effect. Requires root; verified on surya MIUI 12
 * (2026-10-09): DisplayFeatureHal "really set fps(120)" + SDM
 * SetActiveConfig -> the panel mode actually changes.
 */
public class RefreshFps {
    public static void main(String[] args) throws Exception {
        int hz = Integer.parseInt(args[0]);
        Class<?> c = Class.forName("miui.hardware.display.DisplayFeatureManager");
        Object mgr = c.getMethod("getInstance").invoke(null);
        c.getMethod("setScreenEffect", int.class, int.class).invoke(mgr, 24, hz);
        System.out.println("fps " + hz);
    }
}
