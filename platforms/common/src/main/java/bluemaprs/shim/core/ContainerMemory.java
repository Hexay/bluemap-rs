package bluemaprs.shim.core;

import bluemaprs.shim.ipc.Proto.MemoryBudget;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.OptionalLong;

/**
 * The container's memory limit and what the JVM leaves of it for the core, which runs outside the heap but inside
 * the same cgroup (docs/15 §2; the core turns {@link MemoryBudget#budgetMib()} into its {@code memory-limit}).
 *
 * <p>JVM non-heap overhead is estimated as {@value #FIXED_OVERHEAD_MIB} MiB plus 1/{@value #HEAP_DIVISOR} of the max
 * heap. The fixed part covers metaspace (a Paper server with plugins commonly holds 100–200 MiB), the used part of
 * the code cache (tens of MiB of its 240 MiB reservation), the touched stacks of a few hundred threads, and NIO/Netty
 * direct buffers and malloc arenas; the heap-proportional part covers G1's remembered sets, card table and marking
 * bitmaps, which grow with the heap. It errs high: an overestimate only makes the core render with fewer threads,
 * an underestimate gets the server OOM-killed.
 */
public final class ContainerMemory {

    static final long FIXED_OVERHEAD_MIB = 256;
    static final long HEAP_DIVISOR = 10;
    /** At or above this a cgroup limit means none: v1 writes {@code LONG_MAX} rounded down to the page size. */
    private static final long UNLIMITED = 1L << 60;
    private static final long MIB = 1 << 20;

    private ContainerMemory() {}

    /** {@code null} without a container memory limit (or off Linux). */
    public static MemoryBudget detect() {
        OptionalLong limit = limitBytes(Path.of("/proc/self/cgroup"), Path.of("/sys/fs/cgroup"));
        return limit.isPresent() ? budget(limit.getAsLong(), Runtime.getRuntime().maxMemory()) : null;
    }

    static MemoryBudget budget(long limitBytes, long maxHeapBytes) {
        long container = limitBytes / MIB;
        long heap = Math.min(maxHeapBytes, limitBytes) / MIB;
        long overhead = FIXED_OVERHEAD_MIB + heap / HEAP_DIVISOR;
        return new MemoryBudget(container, heap, overhead, container - heap - overhead);
    }

    /**
     * The smallest memory limit on the path from this process's cgroup up to the mount root, cgroup v2
     * ({@code memory.max}) and v1 ({@code memory/memory.limit_in_bytes}). Walking up covers a namespaced container
     * (its cgroup is the root), a host-visible one ({@code /system.slice/docker-….scope}) and a v1 path that does
     * not exist inside the container's mount.
     */
    static OptionalLong limitBytes(Path procSelfCgroup, Path mount) {
        List<String> lines;
        try {
            lines = Files.readAllLines(procSelfCgroup);
        } catch (IOException e) {
            return OptionalLong.empty();
        }
        long min = Long.MAX_VALUE;
        for (String line : lines) {
            String[] parts = line.split(":", 3);
            if (parts.length != 3) continue;
            if (parts[0].equals("0") && parts[1].isEmpty()) {
                min = Math.min(min, smallestUpwards(mount, parts[2], "memory.max"));
            } else if (List.of(parts[1].split(",")).contains("memory")) {
                min = Math.min(min, smallestUpwards(mount.resolve("memory"), parts[2], "memory.limit_in_bytes"));
            }
        }
        return min < UNLIMITED ? OptionalLong.of(min) : OptionalLong.empty();
    }

    private static long smallestUpwards(Path root, String cgroup, String file) {
        long min = Long.MAX_VALUE;
        Path dir = root.resolve(cgroup.replaceFirst("^/+", "")).normalize();
        while (dir != null && dir.startsWith(root)) {
            min = Math.min(min, read(dir.resolve(file)));
            dir = dir.getParent();
        }
        return min;
    }

    /** {@code Long.MAX_VALUE} for "max", a missing or unreadable file, or garbage. */
    static long read(Path file) {
        try {
            String value = Files.readString(file).trim();
            long bytes = value.equals("max") ? Long.MAX_VALUE : Long.parseLong(value);
            return bytes > 0 ? bytes : Long.MAX_VALUE;
        } catch (IOException | NumberFormatException e) {
            return Long.MAX_VALUE;
        }
    }

}
