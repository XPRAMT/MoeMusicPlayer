import java.io.File
import org.apache.tools.ant.taskdefs.condition.Os
import org.gradle.api.DefaultTask
import org.gradle.api.GradleException
import org.gradle.api.logging.LogLevel
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.TaskAction
import javax.inject.Inject
import org.gradle.process.ExecOperations

abstract class BuildTask : DefaultTask() {
    @get:Inject
    abstract val execOperations: ExecOperations

    @Input
    var rootDirRel: String? = null
    @Input
    var projectDir: String? = null
    @Input
    var target: String? = null
    @Input
    var release: Boolean? = null

    @TaskAction
    fun assemble() {
        val windows = Os.isFamily(Os.FAMILY_WINDOWS)
        runTauriCli(if (windows) "cmd.exe" else "npm", windows)
    }

    fun runTauriCli(executable: String, windows: Boolean) {
        val rootDirRel = rootDirRel ?: throw GradleException("rootDirRel cannot be null")
        val target = target ?: throw GradleException("target cannot be null")
        val release = release ?: throw GradleException("release cannot be null")
        val tauriArgs = listOf("run", "--", "tauri", "android", "android-studio-script")

        execOperations.exec {
            workingDir(File(projectDir, rootDirRel))
            executable(executable)
            if (windows) {
                args(listOf("/d", "/c", "npm.cmd") + tauriArgs)
            } else {
                args(tauriArgs)
            }
            if (logger.isEnabled(LogLevel.DEBUG)) {
                args("-vv")
            } else if (logger.isEnabled(LogLevel.INFO)) {
                args("-v")
            }
            if (release) {
                args("--release")
            }
            args(listOf("--target", target))
        }.assertNormalExitValue()
    }
}
