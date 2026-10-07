package bluemaprs.fabric;

import bluemaprs.shim.commands.CommandBridge;
import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.mojang.serialization.JsonOps;
import me.lucko.fabric.api.permissions.v0.Permissions;
import net.minecraft.commands.CommandSourceStack;
import net.minecraft.network.chat.ComponentSerialization;
import net.minecraft.server.permissions.PermissionLevel;
import net.minecraft.world.phys.Vec3;

/**
 * A Fabric command source for {@link CommandBridge} (upstream {@code FabricCommandSource}): permission nodes through
 * fabric-permissions-api, falling back to the moderator op level.
 */
record FabricSender(CommandSourceStack source) implements CommandBridge.Sender {

    @Override
    public String kind() {
        return console() ? "console" : source.isPlayer() ? "player" : "other";
    }

    @Override
    public String name() {
        return source.getTextName();
    }

    @Override
    public String world() {
        return console() || source.getLevel() == null ? null : FabricPlatform.worldId(source.getLevel());
    }

    @Override
    public double[] position() {
        Vec3 pos = console() ? null : source.getPosition();
        return pos == null ? null : new double[] {pos.x, pos.y, pos.z};
    }

    @Override
    public boolean hasPermission(String node) {
        return Permissions.check(source, node, PermissionLevel.MODERATORS);
    }

    @Override
    public void send(JsonElement component) {
        source.sendSystemMessage(ComponentSerialization.CODEC.parse(JsonOps.INSTANCE, padded(component)).getOrThrow());
    }

    private boolean console() {
        return !source.isPlayer() && source.getTextName().equals("Server");
    }

    // upstream's FabricCommandSource frames multi-line messages with a newline before and after
    private static JsonElement padded(JsonElement component) {
        if (!component.toString().contains("\\n")) return component;
        JsonArray extra = new JsonArray();
        extra.add(component);
        extra.add("\n");
        JsonObject framed = new JsonObject();
        framed.addProperty("text", "\n");
        framed.add("extra", extra);
        return framed;
    }

}
