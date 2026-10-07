package bluemaprs.fabric;

import bluemaprs.shim.players.PlayerRegistry;
import bluemaprs.shim.players.PlayerSnapshot;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.effect.MobEffectInstance;
import net.minecraft.world.effect.MobEffects;
import net.minecraft.world.level.LightLayer;

import java.util.HashSet;
import java.util.List;
import java.util.Set;
import java.util.UUID;

/** Server-thread player snapshots, round-robin so each player refreshes about once a second (upstream {@code FabricMod}). */
final class FabricPlayers {

    private final PlayerRegistry registry;
    private int index;

    FabricPlayers(PlayerRegistry registry) {
        this.registry = registry;
    }

    void join(ServerPlayer player) {
        registry.join(snapshot(player));
    }

    void leave(ServerPlayer player) {
        registry.leave(player.getUUID());
    }

    /** {@code END_SERVER_TICK}: {@code max(1, n/20)} players per tick. */
    void tick(MinecraftServer server) {
        List<ServerPlayer> players = server.getPlayerList().getPlayers();
        // Carpet's fake players leave without a DISCONNECT event
        if (server.getTickCount() % 20 == 0) {
            Set<UUID> online = new HashSet<>();
            players.forEach(p -> online.add(p.getUUID()));
            registry.retain(online);
        }
        int count = players.size();
        if (count == 0) return;
        for (int i = Math.max(1, count / 20); i > 0; i--) {
            index++;
            if (index >= 20 && index >= count) index = 0;
            if (index < count) registry.update(snapshot(players.get(index)));
        }
    }

    /** Upstream {@code FabricPlayer.update}: head yaw, light at the feet block, no vanish source on Fabric. */
    private static PlayerSnapshot snapshot(ServerPlayer player) {
        ServerLevel level = player.level();
        MobEffectInstance invisibility = player.getEffect(MobEffects.INVISIBILITY);
        return new PlayerSnapshot(
                player.getUUID(),
                player.getName().getString(),
                FabricPlatform.worldId(level),
                player.getX(), player.getY(), player.getZ(),
                player.getXRot(), player.getYHeadRot(),
                level.getLightEngine().getLayerListener(LightLayer.SKY).getLightValue(player.blockPosition()),
                level.getLightEngine().getLayerListener(LightLayer.BLOCK).getLightValue(player.blockPosition()),
                player.isShiftKeyDown(),
                invisibility != null && invisibility.getDuration() > 0,
                false,
                switch (player.gameMode.getGameModeForPlayer()) {
                    case CREATIVE -> "creative";
                    case ADVENTURE -> "adventure";
                    case SPECTATOR -> "spectator";
                    default -> "survival";
                }
        );
    }

}
