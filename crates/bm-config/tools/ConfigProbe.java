// Oracle for bm-config tests: runs BlueMap's real Configurate stack (from the BlueMap CLI jar).
// Usage: java -cp bluemap-5.28-cli.jar ConfigProbe.java raw <file.conf>...           -> node tree as JSON (or ERROR: ...)
//        java -cp bluemap-5.28-cli.jar ConfigProbe.java expect <file.conf>...        -> same, written to <file>.expected
//        java -cp bluemap-5.28-cli.jar ConfigProbe.java typed <ConfigClass> <file>... -> typed fields after object mapping
import de.bluecolored.bluemap.common.config.ConfigManager;
import org.spongepowered.configurate.ConfigurationNode;
import org.spongepowered.configurate.gson.GsonConfigurationLoader;
import org.spongepowered.configurate.hocon.HoconConfigurationLoader;
import org.spongepowered.configurate.loader.HeaderMode;

import java.lang.reflect.Field;
import java.lang.reflect.Modifier;
import java.nio.file.Files;
import java.nio.file.Path;

public class ConfigProbe {
    public static void main(String[] args) throws Exception {
        if (args[0].equals("raw")) {
            for (int i = 1; i < args.length; i++) System.out.println(raw(Path.of(args[i])));
        } else if (args[0].equals("expect")) {
            for (int i = 1; i < args.length; i++) {
                Path in = Path.of(args[i]);
                Path out = in.resolveSibling(in.getFileName().toString().replaceAll("\\.conf$", ".expected"));
                Files.writeString(out, raw(in) + "\n");
            }
        } else {
            Class<?> type = Class.forName("de.bluecolored.bluemap.common.config." + args[1]);
            for (int i = 2; i < args.length; i++) {
                Path file = Path.of(args[i]);
                try {
                    Object cfg = new ConfigManager(file.getParent()).loadConfig(file, type);
                    System.out.println(file.getFileName() + " " + dump(cfg));
                } catch (Exception e) {
                    System.out.println(file.getFileName() + " ERROR: " + chain(e));
                }
            }
        }
    }

    static String raw(Path file) {
        try {
            ConfigurationNode node = HoconConfigurationLoader.builder().path(file).build().load();
            return GsonConfigurationLoader.builder().headerMode(HeaderMode.NONE).indent(0).buildAndSaveString(node).strip();
        } catch (Exception e) {
            return "ERROR: " + chain(e);
        }
    }

    static String chain(Throwable e) {
        StringBuilder sb = new StringBuilder();
        for (Throwable t = e; t != null; t = t.getCause()) sb.append(t.getClass().getSimpleName()).append(": ").append(t.getMessage()).append(" | ");
        return sb.toString().replace('\n', ' ');
    }

    static String dump(Object o) throws IllegalAccessException {
        StringBuilder sb = new StringBuilder("{");
        for (Class<?> c = o.getClass(); c != Object.class; c = c.getSuperclass()) {
            for (Field f : c.getDeclaredFields()) {
                if (Modifier.isStatic(f.getModifiers())) continue;
                f.setAccessible(true);
                Object v = f.get(o);
                sb.append(f.getName()).append('=').append(v instanceof ConfigurationNode n ? n.raw() : v).append("; ");
            }
        }
        return sb.append('}').toString();
    }
}
