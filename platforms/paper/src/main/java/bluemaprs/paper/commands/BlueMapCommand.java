package bluemaprs.paper.commands;

import bluemaprs.paper.api.Mirror;
import bluemaprs.paper.ipc.CoreLink;
import bluemaprs.paper.ipc.Frame;
import bluemaprs.paper.ipc.Msg;
import bluemaprs.paper.ipc.Proto;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.mojang.brigadier.arguments.StringArgumentType;
import com.mojang.brigadier.context.CommandContext;
import com.mojang.brigadier.suggestion.Suggestions;
import com.mojang.brigadier.suggestion.SuggestionsBuilder;
import io.papermc.paper.command.brigadier.CommandSourceStack;
import io.papermc.paper.command.brigadier.Commands;
import net.kyori.adventure.text.Component;
import net.kyori.adventure.text.format.NamedTextColor;
import net.kyori.adventure.text.serializer.gson.GsonComponentSerializer;
import org.bukkit.Location;
import org.bukkit.World;
import org.bukkit.command.CommandSender;
import org.bukkit.command.ConsoleCommandSender;
import org.bukkit.entity.Player;

import java.io.IOException;
import java.util.Collection;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ConcurrentHashMap;
import java.util.function.Supplier;

/**
 * {@code /bluemap [args…]}: Brigadier shell over the core's parser. Gated on holding any node of commands.json;
 * the core checks the node of the usage it matched. Output arrives as {@code CommandOutput} frames.
 */
@SuppressWarnings("UnstableApiUsage")
public final class BlueMapCommand {

    private record Pending(CommandSender sender, String args) {}

    private final CommandSpec spec;
    private final CoreLink link;
    private final Supplier<Mirror> mirror;
    private final Supplier<CoreControl> control;
    private final String shimVersion;
    private final Map<Long, Pending> pending = new ConcurrentHashMap<>();

    public BlueMapCommand(CommandSpec spec, CoreLink link, Supplier<Mirror> mirror, Supplier<CoreControl> control,
                          String shimVersion) {
        this.spec = spec;
        this.link = link;
        this.mirror = mirror;
        this.control = control;
        this.shimVersion = shimVersion;
    }

    public void register(Commands commands) {
        commands.register(Commands.literal(spec.root())
                .requires(source -> spec.permissions().stream().anyMatch(source.getSender()::hasPermission))
                .executes(ctx -> execute(ctx.getSource(), ""))
                .then(Commands.argument("args", StringArgumentType.greedyString())
                        .suggests(this::suggest)
                        .executes(ctx -> execute(ctx.getSource(), StringArgumentType.getString(ctx, "args"))))
                .build());
    }

    public void output(long id, JsonElement component) {
        Pending p = pending.get(id);
        if (p != null) p.sender().sendMessage(GsonComponentSerializer.gson().deserialize(component.toString()).compact());
    }

    public void done(long id) {
        Pending p = pending.remove(id);
        if (p != null && p.args().equalsIgnoreCase("version"))
            p.sender().sendMessage(Component.text("bluemap-rs Paper shim " + shimVersion, NamedTextColor.GRAY));
    }

    /** The core died: commands still running will never finish. */
    public void abortAll() {
        pending.values().forEach(p -> p.sender().sendMessage(
                Component.text("The BlueMap core stopped before this command finished.", NamedTextColor.RED)));
        pending.clear();
    }

    private int execute(CommandSourceStack source, String rawArgs) {
        CommandSender sender = source.getSender();
        String args = rawArgs.trim();
        if (!link.connected()) return coreDown(sender, args);

        long id = link.nextId();
        JsonObject header = Msg.of("Command");
        header.addProperty("id", id);
        header.addProperty("input", args.isEmpty() ? spec.root() : spec.root() + " " + args);
        header.add("sender", Msg.GSON.toJsonTree(describe(source)));
        pending.put(id, new Pending(sender, args));
        try {
            link.send(new Frame(header));
        } catch (IOException e) {
            pending.remove(id);
            return coreDown(sender, args);
        }
        return 1;
    }

    private int coreDown(CommandSender sender, String args) {
        CoreControl core = control.get();
        boolean reload = args.equalsIgnoreCase("reload") || args.equalsIgnoreCase("reload light");
        if (reload && core != null && sender.hasPermission(permissionOf(args))) {
            sender.sendMessage(Component.text("Starting the BlueMap core…", NamedTextColor.GOLD));
            core.restartNow();
            return 1;
        }
        String hint = core != null && core.willRestart() ? "(restarting…)" : "(run /bluemap reload to start it)";
        sender.sendMessage(Component.text("BlueMap core is not running " + hint, NamedTextColor.RED));
        return 0;
    }

    private String permissionOf(String args) {
        return args.equalsIgnoreCase("reload light") ? "bluemap.reload.light" : "bluemap.reload";
    }

    private Proto.CommandSender describe(CommandSourceStack source) {
        CommandSender sender = source.getSender();
        String kind = sender instanceof ConsoleCommandSender ? "console" : sender instanceof Player ? "player" : "other";
        List<String> held = spec.permissions().stream().filter(sender::hasPermission).toList();
        String world = null;
        double[] position = null;
        if (!(sender instanceof ConsoleCommandSender)) {
            try {
                Location location = source.getLocation();
                World w = location.getWorld();
                if (w != null) world = w.getKey().toString();
                position = new double[] {location.getX(), location.getY(), location.getZ()};
            } catch (NullPointerException ignored) {
                // https://github.com/PaperMC/Paper/issues/13387, as upstream
            }
        }
        return new Proto.CommandSender(kind, sender.getName(), world, position, held);
    }

    private CompletableFuture<Suggestions> suggest(CommandContext<CommandSourceStack> ctx, SuggestionsBuilder builder) {
        String remaining = builder.getRemaining();
        int lastSpace = remaining.lastIndexOf(' ');
        List<String> done = CommandSpec.tokenize(lastSpace < 0 ? "" : remaining.substring(0, lastSpace));
        String partial = remaining.substring(lastSpace + 1);
        CommandSender sender = ctx.getSource().getSender();

        SuggestionsBuilder offset = builder.createOffset(builder.getStart() + lastSpace + 1);
        spec.suggest(done, partial, sender::hasPermission, this::placeholderValues).forEach(offset::suggest);
        return offset.buildFuture();
    }

    private Collection<String> placeholderValues(String placeholder) {
        Mirror m = mirror.get();
        return switch (placeholder) {
            case "<map>" -> m.mapIds();
            case "<storage>" -> m.storages();
            default -> List.of();
        };
    }

}
