package bluemaprs.paper.commands;

import bluemaprs.paper.ipc.Msg;

import java.io.IOException;
import java.io.InputStream;
import java.io.InputStreamReader;
import java.io.Reader;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collection;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Locale;
import java.util.Set;
import java.util.function.Function;
import java.util.function.Predicate;

/** {@code commands.json} (single source in bm-cli): every {@code /bluemap} usage with its permission node. */
public final class CommandSpec {

    public record Usage(List<String> tokens, String permission) {}

    private record Json(String root, List<Entry> commands) {}

    private record Entry(String usage, String permission) {}

    private final String root;
    private final List<Usage> usages;
    private final List<String> permissions;

    CommandSpec(String root, List<Usage> usages) {
        this.root = root;
        this.usages = List.copyOf(usages);
        this.permissions = usages.stream().map(Usage::permission).distinct().toList();
    }

    public static CommandSpec load(InputStream in) throws IOException {
        if (in == null) throw new IOException("commands.json is missing from the plugin jar");
        try (Reader reader = new InputStreamReader(in, StandardCharsets.UTF_8)) {
            Json json = Msg.GSON.fromJson(reader, Json.class);
            List<Usage> usages = new ArrayList<>();
            for (Entry e : json.commands()) usages.add(new Usage(tokenize(e.usage()), e.permission()));
            return new CommandSpec(json.root(), usages);
        }
    }

    public String root() {
        return root;
    }

    /** Distinct permission nodes in file order. */
    public List<String> permissions() {
        return permissions;
    }

    /**
     * Candidates for the token after {@code done}, starting with {@code partial}; {@code <placeholder>} tokens
     * match any input and expand through {@code placeholderValues}.
     */
    public List<String> suggest(List<String> done, String partial, Predicate<String> hasPermission,
                                Function<String, Collection<String>> placeholderValues) {
        String prefix = partial.toLowerCase(Locale.ROOT);
        Set<String> out = new LinkedHashSet<>();
        for (Usage usage : usages) {
            if (usage.tokens().size() <= done.size() || !matches(usage, done) || !hasPermission.test(usage.permission()))
                continue;
            String next = usage.tokens().get(done.size());
            Collection<String> candidates = isPlaceholder(next) ? placeholderValues.apply(next) : List.of(next);
            for (String candidate : candidates) {
                if (candidate.toLowerCase(Locale.ROOT).startsWith(prefix)) out.add(candidate);
            }
        }
        return List.copyOf(out);
    }

    static List<String> tokenize(String input) {
        String trimmed = input.trim();
        return trimmed.isEmpty() ? List.of() : Arrays.asList(trimmed.split("\\s+"));
    }

    private static boolean matches(Usage usage, List<String> done) {
        for (int i = 0; i < done.size(); i++) {
            String token = usage.tokens().get(i);
            if (!isPlaceholder(token) && !token.equalsIgnoreCase(done.get(i))) return false;
        }
        return true;
    }

    private static boolean isPlaceholder(String token) {
        return token.startsWith("<") && token.endsWith(">");
    }

}
