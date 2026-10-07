package bluemaprs.paper.commands;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

class CommandSpecTest {

    private static CommandSpec spec() throws IOException {
        return CommandSpec.load(CommandSpecTest.class.getResourceAsStream("/commands.json"));
    }

    @Test
    void loadsTheSharedCommandsJson() throws IOException {
        CommandSpec spec = spec();
        assertEquals("bluemap", spec.root());
        assertTrue(spec.permissions().contains("bluemap.reload.light"));
        assertEquals(spec.permissions().stream().distinct().toList(), spec.permissions());
    }

    @Test
    void suggestsLiteralsAndPlaceholders() throws IOException {
        CommandSpec spec = spec();
        List<String> maps = List.of("world", "nether");
        assertEquals(List.of("freeze"), spec.suggest(List.of(), "fr", p -> true, p -> List.of()));
        assertEquals(List.of("world"), spec.suggest(List.of("freeze"), "w", p -> true, p -> maps));
        assertEquals(List.of("delete"),
                spec.suggest(List.of("storages", "file"), "", p -> true, p -> List.of()));
        assertEquals(List.of(), spec.suggest(List.of(), "fr", p -> !p.equals("bluemap.freeze"), p -> List.of()));
    }

    @Test
    void tokenizeIgnoresExtraSpaces() {
        assertEquals(List.of("a", "b"), CommandSpec.tokenize("  a   b "));
        assertEquals(List.of(), CommandSpec.tokenize("   "));
    }

}
