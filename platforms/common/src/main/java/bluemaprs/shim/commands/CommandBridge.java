package bluemaprs.shim.commands;

import bluemaprs.shim.api.Mirror;
import bluemaprs.shim.ipc.CoreLink;
import bluemaprs.shim.ipc.Frame;
import bluemaprs.shim.ipc.Msg;
import bluemaprs.shim.ipc.Proto;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.mojang.brigadier.arguments.StringArgumentType;
import com.mojang.brigadier.builder.LiteralArgumentBuilder;
import com.mojang.brigadier.builder.RequiredArgumentBuilder;
import com.mojang.brigadier.context.CommandContext;
import com.mojang.brigadier.suggestion.Suggestions;
import com.mojang.brigadier.suggestion.SuggestionsBuilder;
import com.mojang.brigadier.tree.LiteralCommandNode;

import java.io.IOException;
import java.util.Collection;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ConcurrentHashMap;
import java.util.function.Function;
import java.util.function.Supplier;

/**
 * {@code /bluemap [args…]}: a Brigadier shell over the core's parser, generic over the platform's command source
 * {@code S}. Gated on holding any node of commands.json; the core checks the node of the usage it matched. Output
 * arrives as {@code CommandOutput} frames.
 */
public final class CommandBridge<S> {

    /** A platform command source as the bridge sees it. */
    public interface Sender {

        /** {@code console|player|other} */
        String kind();

        String name();

        /** {@code WorldInfo.id}, or null (console). */
        String world();

        /** {@code [x, y, z]}, or null (console). */
        double[] position();

        boolean hasPermission(String node);

        /** An Adventure-JSON text component. */
        void send(JsonElement component);

    }

    private record Pending(Sender sender, String args) {}

    private final CommandSpec spec;
    private final CoreLink link;
    private final Supplier<Mirror> mirror;
    private final Supplier<CoreControl> control;
    private final String shimName;
    private final Function<S, Sender> senders;
    private final Map<Long, Pending> pending = new ConcurrentHashMap<>();

    /** @param shimName shown by {@code /bluemap version}, e.g. {@code "Paper shim 5.28+rs.0.1.0"} */
    public CommandBridge(CommandSpec spec, CoreLink link, Supplier<Mirror> mirror, Supplier<CoreControl> control,
                         String shimName, Function<S, Sender> senders) {
        this.spec = spec;
        this.link = link;
        this.mirror = mirror;
        this.control = control;
        this.shimName = shimName;
        this.senders = senders;
    }

    public LiteralCommandNode<S> node() {
        return LiteralArgumentBuilder.<S>literal(spec.root())
                .requires(source -> spec.permissions().stream().anyMatch(senders.apply(source)::hasPermission))
                .executes(ctx -> execute(ctx.getSource(), ""))
                .then(RequiredArgumentBuilder.<S, String>argument("args", StringArgumentType.greedyString())
                        .suggests(this::suggest)
                        .executes(ctx -> execute(ctx.getSource(), StringArgumentType.getString(ctx, "args"))))
                .build();
    }

    public void output(long id, JsonElement component) {
        Pending p = pending.get(id);
        if (p != null) p.sender().send(component);
    }

    public void done(long id) {
        Pending p = pending.remove(id);
        if (p != null && p.args().equalsIgnoreCase("version")) p.sender().send(text("bluemap-rs " + shimName, "gray"));
    }

    /** The core died: commands still running will never finish. */
    public void abortAll() {
        pending.values().forEach(p -> p.sender().send(text("The BlueMap core stopped before this command finished.", "red")));
        pending.clear();
    }

    private int execute(S source, String rawArgs) {
        Sender sender = senders.apply(source);
        String args = rawArgs.trim();
        if (!link.connected()) return coreDown(sender, args);

        long id = link.nextId();
        JsonObject header = Msg.of("Command");
        header.addProperty("id", id);
        header.addProperty("input", args.isEmpty() ? spec.root() : spec.root() + " " + args);
        header.add("sender", Msg.GSON.toJsonTree(describe(sender)));
        pending.put(id, new Pending(sender, args));
        try {
            link.send(new Frame(header));
        } catch (IOException e) {
            pending.remove(id);
            return coreDown(sender, args);
        }
        return 1;
    }

    private int coreDown(Sender sender, String args) {
        CoreControl core = control.get();
        boolean reload = args.equalsIgnoreCase("reload") || args.equalsIgnoreCase("reload light");
        if (reload && core != null && sender.hasPermission(permissionOf(args))) {
            sender.send(text("Starting the BlueMap core…", "gold"));
            core.restartNow();
            return 1;
        }
        String hint = core != null && core.willRestart() ? "(restarting…)" : "(run /bluemap reload to start it)";
        sender.send(text("BlueMap core is not running " + hint, "red"));
        return 0;
    }

    private static String permissionOf(String args) {
        return args.equalsIgnoreCase("reload light") ? "bluemap.reload.light" : "bluemap.reload";
    }

    private Proto.CommandSender describe(Sender sender) {
        List<String> held = spec.permissions().stream().filter(sender::hasPermission).toList();
        return new Proto.CommandSender(sender.kind(), sender.name(), sender.world(), sender.position(), held);
    }

    private CompletableFuture<Suggestions> suggest(CommandContext<S> ctx, SuggestionsBuilder builder) {
        String remaining = builder.getRemaining();
        int lastSpace = remaining.lastIndexOf(' ');
        List<String> done = CommandSpec.tokenize(lastSpace < 0 ? "" : remaining.substring(0, lastSpace));
        String partial = remaining.substring(lastSpace + 1);
        Sender sender = senders.apply(ctx.getSource());

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

    private static JsonObject text(String text, String color) {
        JsonObject component = new JsonObject();
        component.addProperty("text", text);
        component.addProperty("color", color);
        return component;
    }

}
