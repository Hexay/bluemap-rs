package bluemaprs.paper.players;

import bluemaprs.paper.ipc.Proto.PlayerInfo;
import org.bukkit.GameMode;
import org.bukkit.Location;
import org.bukkit.block.Block;
import org.bukkit.entity.Player;
import org.bukkit.metadata.MetadataValue;
import org.bukkit.potion.PotionEffectType;

import java.util.UUID;

/** Upstream {@code BukkitPlayer.update}; must be taken on the player's (entity scheduler) thread. */
record PlayerSnapshot(UUID uuid, String accountName, String world, double x, double y, double z, double pitch,
                      double yaw, int skyLight, int blockLight, boolean sneaking, boolean invisible,
                      boolean vanished, String gamemode) {

    static PlayerSnapshot of(Player player) {
        boolean vanished = false;
        for (MetadataValue meta : player.getMetadata("vanished")) {
            if (meta.asBoolean()) vanished = true;
        }

        Location location = player.getLocation();
        Block block = location.getBlock();
        return new PlayerSnapshot(
                player.getUniqueId(),
                player.getName(),
                player.getWorld().getKey().toString(),
                location.getX(), location.getY(), location.getZ(),
                // widened floats, as upstream's Vector3d(pitch, yaw, 0)
                (double) location.getPitch(), (double) location.getYaw(),
                block.getLightFromSky(), block.getLightFromBlocks(),
                player.isSneaking(),
                player.hasPotionEffect(PotionEffectType.INVISIBILITY),
                vanished,
                gamemode(player.getGameMode())
        );
    }

    PlayerInfo toInfo(String displayName) {
        return new PlayerInfo(uuid.toString(), displayName, world, x, y, z, pitch, yaw, skyLight, blockLight,
                sneaking, invisible, vanished, gamemode);
    }

    // equality, not a switch: GameMode may stop being an enum on newer API versions
    private static String gamemode(GameMode mode) {
        if (mode == GameMode.CREATIVE) return "creative";
        if (mode == GameMode.ADVENTURE) return "adventure";
        if (mode == GameMode.SPECTATOR) return "spectator";
        return "survival";
    }

}
