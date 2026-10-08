package bluemaprs.fabric;

import net.fabricmc.api.ClientModInitializer;
import org.slf4j.LoggerFactory;

/**
 * The client entrypoint: bluemap-rs runs on dedicated servers only (docs/16), so singleplayer and LAN worlds never
 * extract or spawn the core. Must not touch any server-side class.
 */
public final class ClientNotice implements ClientModInitializer {

    @Override
    public void onInitializeClient() {
        LoggerFactory.getLogger("BlueMap").info("bluemap-rs runs on dedicated servers only; it stays inactive in this "
                + "client (singleplayer and LAN worlds are not mapped).");
    }

}
