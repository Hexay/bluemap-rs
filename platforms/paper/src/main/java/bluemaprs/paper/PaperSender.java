package bluemaprs.paper;

import bluemaprs.shim.commands.CommandBridge;
import com.google.gson.JsonElement;
import io.papermc.paper.command.brigadier.CommandSourceStack;
import net.kyori.adventure.text.serializer.gson.GsonComponentSerializer;
import org.bukkit.Location;
import org.bukkit.World;
import org.bukkit.command.CommandSender;
import org.bukkit.command.ConsoleCommandSender;
import org.bukkit.entity.Player;

/** A Paper command source for {@link CommandBridge}. */
@SuppressWarnings("UnstableApiUsage")
record PaperSender(CommandSourceStack source) implements CommandBridge.Sender {

    @Override
    public String kind() {
        CommandSender sender = source.getSender();
        return sender instanceof ConsoleCommandSender ? "console" : sender instanceof Player ? "player" : "other";
    }

    @Override
    public String name() {
        return source.getSender().getName();
    }

    @Override
    public String world() {
        Location location = location();
        World world = location == null ? null : location.getWorld();
        return world == null ? null : world.getKey().toString();
    }

    @Override
    public double[] position() {
        Location location = location();
        return location == null ? null : new double[] {location.getX(), location.getY(), location.getZ()};
    }

    @Override
    public boolean hasPermission(String node) {
        return source.getSender().hasPermission(node);
    }

    @Override
    public void send(JsonElement component) {
        source.getSender().sendMessage(GsonComponentSerializer.gson().deserialize(component.toString()).compact());
    }

    private Location location() {
        if (source.getSender() instanceof ConsoleCommandSender) return null;
        try {
            return source.getLocation();
        } catch (NullPointerException ignored) {
            // https://github.com/PaperMC/Paper/issues/13387, as upstream
            return null;
        }
    }

}
