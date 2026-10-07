package bluemaprs.shim.players;

import bluemaprs.shim.ipc.Proto.PlayerInfo;

import java.util.UUID;

/**
 * Upstream's {@code Player.update()} state; platforms take it on the thread that owns the player.
 *
 * @param world    {@code WorldInfo.id} of the player's world
 * @param gamemode {@code survival|creative|adventure|spectator}
 */
public record PlayerSnapshot(UUID uuid, String accountName, String world, double x, double y, double z,
                             double pitch, double yaw, int skyLight, int blockLight, boolean sneaking,
                             boolean invisible, boolean vanished, String gamemode) {

    PlayerInfo toInfo(String displayName) {
        return new PlayerInfo(uuid.toString(), displayName, world, x, y, z, pitch, yaw, skyLight, blockLight,
                sneaking, invisible, vanished, gamemode);
    }

}
