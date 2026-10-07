package com.mifinetune.core

import org.json.JSONObject

/**
 * Typed client for the `miui-ft` binary (all commands answer JSON on stdout).
 *
 * Responsibility: command dispatch + JSON parsing.
 * Non-goals: process lifecycle (RootBridge), UI state (ViewModel).
 */
class FtClient(private val bridge: RootBridge) {

    class FtException(message: String) : Exception(message)

    /** Runs `miui-ft <args>`; throws [FtException] on non-JSON or crash. */
    fun exec(args: String): JSONObject {
        val r = bridge.sh("${RootBridge.BIN} $args")
        val raw = r.out.trim()
        val start = raw.indexOf('{')
        if (start < 0) {
            throw FtException("miui-ft $args -> exit ${r.code}: ${raw.ifEmpty { "(no output)" }}")
        }
        // stderr noise (if any) precedes the JSON payload; parse from first '{'
        val json = try {
            JSONObject(raw.substring(start))
        } catch (e: Exception) {
            throw FtException("miui-ft $args -> bad JSON: ${e.message}\n$raw")
        }
        return json
    }

    fun status(): Status = parseStatus(exec("status"))

    fun plan(profileId: String): Plan = parsePlan(exec("plan $profileId"))

    fun apply(profileId: String): ApplyReport = parseReport(exec("apply $profileId"))

    fun verify(profileId: String): ApplyReport = parseReport(exec("verify $profileId"))

    fun restore(): ApplyReport = parseReport(exec("restore"))
}
