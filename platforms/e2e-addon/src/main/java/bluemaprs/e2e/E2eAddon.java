package bluemaprs.e2e;

import de.bluecolored.bluemap.api.BlueMapAPI;
import de.bluecolored.bluemap.api.BlueMapMap;
import de.bluecolored.bluemap.api.markers.MarkerSet;
import de.bluecolored.bluemap.api.markers.POIMarker;
import org.bukkit.plugin.java.JavaPlugin;

import java.awt.image.BufferedImage;
import java.util.Optional;

/** Runs the same on our shim and on upstream BlueMap, so the e2e can compare what both write. */
public final class E2eAddon extends JavaPlugin {

    public static final String MARKER_SET = "e2e-addon";

    @Override
    public void onEnable() {
        BlueMapAPI.onEnable(api -> {
            api.getPlugin().setSkinProvider(uuid -> Optional.of(skin()));
            for (BlueMapMap map : api.getMaps()) {
                MarkerSet set = MarkerSet.builder().label("e2e addon").build();
                set.put("poi", POIMarker.builder().label("e2e poi").position(1.5, 64.0, -2.5).build());
                map.getMarkerSets().put(MARKER_SET, set);
            }
        });
    }

    /** The same 64x64 skin for every player (bot UUIDs may differ between runs); the hat layer mixes alpha 0/128/255. */
    private static BufferedImage skin() {
        BufferedImage skin = new BufferedImage(64, 64, BufferedImage.TYPE_INT_ARGB);
        for (int y = 0; y < 64; y++) {
            for (int x = 0; x < 64; x++) {
                int rgb = (x * 0x9E3779B1 + y * 0x85EBCA6B) >>> 8;
                boolean hat = x >= 32 && y < 16;
                int alpha = hat ? new int[]{0, 128, 255}[(x + y) % 3] : 255;
                skin.setRGB(x, y, alpha << 24 | rgb & 0xFFFFFF);
            }
        }
        return skin;
    }

}
