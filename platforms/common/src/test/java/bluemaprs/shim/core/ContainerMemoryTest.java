package bluemaprs.shim.core;

import bluemaprs.shim.ipc.Proto.MemoryBudget;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.OptionalLong;

import static org.junit.jupiter.api.Assertions.assertEquals;

class ContainerMemoryTest {

    private static final long GIB = 1L << 30;

    @TempDir
    Path tmp;

    private Path write(String relative, String content) throws IOException {
        Path file = tmp.resolve(relative);
        Files.createDirectories(file.getParent());
        Files.writeString(file, content);
        return file;
    }

    private OptionalLong limit(String procSelfCgroup) throws IOException {
        return ContainerMemory.limitBytes(write("proc/self/cgroup", procSelfCgroup), tmp.resolve("sys"));
    }

    @Test
    void v2NamespacedContainer() throws IOException {
        write("sys/memory.max", "4294967296\n");
        assertEquals(OptionalLong.of(4 * GIB), limit("0::/\n"));
    }

    @Test
    void v2MaxMeansNoLimit() throws IOException {
        write("sys/memory.max", "max\n");
        assertEquals(OptionalLong.empty(), limit("0::/\n"));
    }

    @Test
    void v2TakesTheSmallestLimitUpTheTree() throws IOException {
        write("sys/system.slice/memory.max", "2147483648\n");
        write("sys/system.slice/docker-abc.scope/memory.max", "max\n");
        assertEquals(OptionalLong.of(2 * GIB), limit("0::/system.slice/docker-abc.scope\n"));
    }

    @Test
    void v1ContainerWhosePathIsMissingFromItsMount() throws IOException {
        write("sys/memory/memory.limit_in_bytes", "1073741824\n");
        assertEquals(OptionalLong.of(GIB), limit("12:cpu,cpuacct:/docker/abc\n4:memory:/docker/abc\n"));
    }

    @Test
    void v1PageRoundedLongMaxMeansNoLimit() throws IOException {
        write("sys/memory/memory.limit_in_bytes", "9223372036854771712\n");
        assertEquals(OptionalLong.empty(), limit("4:memory:/\n"));
    }

    @Test
    void noCgroupFilesMeansNoLimit() throws IOException {
        assertEquals(OptionalLong.empty(), ContainerMemory.limitBytes(tmp.resolve("missing"), tmp));
        assertEquals(OptionalLong.empty(), limit("0::/\n"));
        write("sys/memory.max", "garbage");
        assertEquals(OptionalLong.empty(), limit("0::/\n"));
    }

    @Test
    void budgetIsWhatTheJvmLeaves() {
        // 3 GiB heap: 256 + 307 MiB overhead
        assertEquals(new MemoryBudget(4096, 3072, 563, 461), ContainerMemory.budget(4 * GIB, 3 * GIB));
    }

    @Test
    void pterodactylsNinetyFivePercentLeavesNothing() {
        long heap = (long) (4 * GIB * 0.95);
        assertEquals(new MemoryBudget(4096, 3891, 645, -440), ContainerMemory.budget(4 * GIB, heap));
    }

    @Test
    void unboundedHeapIsCappedAtTheContainer() {
        assertEquals(new MemoryBudget(1024, 1024, 358, -358), ContainerMemory.budget(GIB, Long.MAX_VALUE));
    }

}
