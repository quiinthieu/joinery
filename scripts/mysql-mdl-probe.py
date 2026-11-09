#!/usr/bin/env python3
"""Bounded public-fixture MDL experiment; no Joinery application safety claim.

Requires PyMySQL==1.1.2. Uses only fixed loopback fixtures and UUID-owned tables.
No account grants, server-wide settings, application state or keychain access.
"""
import argparse
import json
import queue
import socket
import ssl
import threading
import time
import uuid
from pathlib import Path

import pymysql


FIXTURES = (("mysql", 53306, "INNODB_TABLES"), ("mariadb", 53307, "INNODB_SYS_TABLES"))
IN_TRANS = 1
AUTOCOMMIT = 2


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def status(connection):
    raw = connection.server_status
    return {"raw": raw, "inTransaction": bool(raw & IN_TRANS), "autocommit": bool(raw & AUTOCOMMIT)}


def sql(connection, statement, parameters=None):
    started = time.monotonic()
    try:
        with connection.cursor() as cursor:
            cursor.execute(statement, parameters)
            rows = cursor.fetchall()
        return {"ok": True, "rows": rows, "seconds": round(time.monotonic() - started, 4), "status": status(connection)}
    except pymysql.MySQLError as error:
        return {"ok": False, "code": error.args[0], "message": str(error.args[1]) if len(error.args) > 1 else str(error), "seconds": round(time.monotonic() - started, 4), "cachedStatus": status(connection)}


def successful(connection, statement, parameters=None):
    result = sql(connection, statement, parameters)
    require(result["ok"], result)
    return result


def background(connection, statement, jobs):
    results = queue.Queue()

    def work():
        try:
            results.put(sql(connection, statement))
        except BaseException as error:
            results.put({"ok": False, "workerFailure": {"type": type(error).__name__, "message": str(error)}})

    thread = threading.Thread(target=work, daemon=True)
    job = (thread, results, connection)
    jobs.append(job)
    thread.start()
    return job


def completed(job):
    thread, results, _connection = job
    thread.join(9)
    require(not thread.is_alive(), "bounded worker did not finish")
    return results.get(timeout=1)


def run_engine(engine, port, dictionary):
    prefix = "mdl_" + uuid.uuid4().hex
    names = [prefix + suffix for suffix in ("_drop", "_rename", "_renamed", "_queue", "_lost", "_handoff", "_writer_zero", "_read_committed")]
    opened = []
    jobs = []
    setup_close_errors = []
    report = {"engine": engine, "port": port, "ownedTables": names, "cases": {}, "cleanup": {"verified": False}}

    def connect(lock_timeout=2, track=True):
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        context.check_hostname = False
        context.verify_mode = ssl.CERT_NONE  # Isolated, self-signed loopback fixture only.
        connection = pymysql.connect(host="127.0.0.1", port=port, user="joinery", password="joinery-local-only", database="joinery", autocommit=True, connect_timeout=3, read_timeout=8, write_timeout=3, ssl=context)
        try:
            successful(connection, "SET SESSION lock_wait_timeout = %s", (lock_timeout,))
            successful(connection, "SET SESSION innodb_lock_wait_timeout = 2")
        except BaseException:
            try:
                connection.close()
            except BaseException as error:
                setup_close_errors.append({"phase": "connectionSetupClose", "type": type(error).__name__, "message": str(error)})
            raise
        if track:
            opened.append(connection)
        return connection

    def create(observer, name):
        successful(observer, "CREATE TABLE `" + name + "` (id INT PRIMARY KEY, note VARCHAR(64) NOT NULL DEFAULT 'original') ENGINE=InnoDB")

    def guard_table(connection, name):
        begin = successful(connection, "START TRANSACTION")
        accessed = successful(connection, "SELECT id FROM `" + name + "` LIMIT 1")
        require(begin["status"]["inTransaction"] and accessed["status"]["inTransaction"], "explicit transaction not active")
        return accessed

    def timeout(result):
        require(not result["ok"] and result["code"] == 1205, result)
        require(result["seconds"] < 7, "lock wait exceeded bound")

    def break_transport(connection):
        # No COM_QUIT or privileged KILL: deliberately lose only this owned socket.
        # Retain the client's open handle and cached transaction state, so the
        # later reconnect=False heartbeat must encounter the broken transport.
        connection._sock.shutdown(socket.SHUT_RDWR)

    observer = None
    try:
        observer = connect()
        report["serverVersion"] = successful(observer, "SELECT VERSION()")["rows"][0][0]
        report["account"] = successful(observer, "SELECT CURRENT_USER()")["rows"][0][0]
        denied = sql(observer, "SELECT TABLE_ID, NAME FROM INFORMATION_SCHEMA.`" + dictionary + "` LIMIT 1")
        require(not denied["ok"] and denied["code"] in (1142, 1227), denied)
        report["dictionaryDenied"] = denied

        probe = connect()
        initial = successful(probe, "SELECT 1")
        begun = successful(probe, "START TRANSACTION")
        inside = successful(probe, "SELECT 1")
        rolled_back = successful(probe, "ROLLBACK")
        afterward = successful(probe, "SELECT 1")
        require(not initial["status"]["inTransaction"] and begun["status"]["inTransaction"] and inside["status"]["inTransaction"] and not rolled_back["status"]["inTransaction"] and not afterward["status"]["inTransaction"], "transaction status not observable")
        report["transactionStatus"] = {"initial": initial["status"], "begin": begun["status"], "heartbeat": inside["status"], "rollback": rolled_back["status"], "afterRollbackHeartbeat": afterward["status"]}
        probe.close()

        for case, name, ddl in (("emptyDrop", names[0], "DROP TABLE `" + names[0] + "`"), ("emptyRename", names[1], "RENAME TABLE `" + names[1] + "` TO `" + names[2] + "`")):
            create(observer, name)
            guard = connect()
            accessed = guard_table(guard, name)
            require(accessed["rows"] == (), "expected empty original table")
            blocked = sql(observer, ddl)
            timeout(blocked)
            rollback = successful(guard, "ROLLBACK")
            released = successful(observer, ddl)
            report["cases"][case] = {"emptyRead": accessed, "ddlWhileGuarded": blocked, "rollback": rollback, "ddlAfterRollback": released}
            guard.close()

        # Qualify the application's exact values.rs projection / catalog.rs
        # writer lock SQL separately from the LIMIT 1 guard premise above.
        # READ COMMITTED is also a separate, dedicated read-only guard experiment.
        for case, name, read_committed in (("exactWriterZeroRows", names[6], False), ("readCommittedGuard", names[7], True)):
            create(observer, name)
            qualified = "`joinery`.`" + name + "`"
            empty = successful(observer, "SELECT COUNT(*) FROM " + qualified)
            require(empty["rows"] == ((0,),), "expected empty owned table")
            guard = connect()
            isolation_sql = "SELECT @@SESSION." + ("transaction_isolation" if engine == "mysql" else "tx_isolation")
            isolation_set = None
            if read_committed:
                isolation_set = successful(guard, "SET SESSION TRANSACTION ISOLATION LEVEL READ COMMITTED")
            isolation = successful(guard, isolation_sql)
            if read_committed:
                require(isolation["rows"] == (("READ-COMMITTED",),), isolation)
            begin_sql = "START TRANSACTION READ ONLY" if read_committed else "START TRANSACTION"
            lock_sql = ("SELECT `id` FROM " + qualified + " LIMIT 1") if read_committed else ("SELECT `id`,`note` FROM " + qualified + " WHERE 1=0 FOR UPDATE")
            begin = successful(guard, begin_sql)
            accessed = successful(guard, lock_sql)
            require(accessed["rows"] == (), "expected zero-row acquisition")
            started = time.monotonic()
            guard.ping(reconnect=False)
            heartbeat = {"command": "COM_PING", "reconnect": False, "seconds": round(time.monotonic() - started, 4), "status": status(guard)}
            require(begin["status"]["inTransaction"] and accessed["status"]["inTransaction"] and heartbeat["status"]["inTransaction"] and heartbeat["status"]["autocommit"], "fresh heartbeat did not prove the explicit transaction remained active")
            ddl_sql = "DROP TABLE " + qualified
            blocked = sql(observer, ddl_sql)
            timeout(blocked)
            rollback = successful(guard, "ROLLBACK")
            require(not rollback["status"]["inTransaction"], rollback)
            released = successful(observer, ddl_sql)
            report["cases"][case] = {"emptyTable": empty, "isolationSet": isolation_set, "isolationSql": isolation_sql, "isolation": isolation, "beginSql": begin_sql, "begin": begin, "lockSql": lock_sql, "emptyRead": accessed, "freshHeartbeat": heartbeat, "ddlSql": ddl_sql, "ddlWhileGuarded": blocked, "rollback": rollback, "ddlAfterRollback": released}
            guard.close()

        create(observer, names[3])
        guard, ddl_connection, writer = connect(), connect(5), connect(2)
        guard_table(guard, names[3])
        ddl_id = ddl_connection.thread_id()
        pending_ddl = background(ddl_connection, "DROP TABLE `" + names[3] + "`", jobs)
        queued = None
        deadline = time.monotonic() + 2
        while time.monotonic() < deadline:
            rows = successful(observer, "SELECT ID, STATE FROM INFORMATION_SCHEMA.PROCESSLIST WHERE ID = %s", (ddl_id,))["rows"]
            if rows and "metadata lock" in str(rows[0][1]).lower():
                queued = rows
                break
            time.sleep(0.02)
        require(queued is not None, "DDL did not enter observed metadata-lock wait")
        successful(writer, "START TRANSACTION")
        writer_attempt = sql(writer, "SELECT id FROM `" + names[3] + "` LIMIT 1")
        if not writer_attempt["ok"]:
            timeout(writer_attempt)
        successful(writer, "ROLLBACK")
        still_guarded = successful(guard, "SELECT 1")
        require(still_guarded["status"]["inTransaction"], "original guard ended unexpectedly")
        successful(guard, "ROLLBACK")
        ddl_result = completed(pending_ddl)
        require(ddl_result["ok"], ddl_result)
        report["cases"]["writerBehindQueuedDdl"] = {"observedDdlWait": queued, "writerAttempt": writer_attempt, "writerBlocked": not writer_attempt["ok"], "guardStillActive": still_guarded["status"], "ddlAfterGuardRollback": ddl_result}
        guard.close()
        writer.close()
        ddl_connection.close()

        create(observer, names[4])
        guard = connect()
        guard_table(guard, names[4])
        cached_before_loss = status(guard)
        break_transport(guard)
        handle_open_after_loss = guard.open
        replaced = successful(observer, "DROP TABLE `" + names[4] + "`")
        create(observer, names[4])
        successful(observer, "INSERT INTO `" + names[4] + "` VALUES (2, 'replacement')")
        writer = connect()
        new_read = guard_table(writer, names[4])
        try:
            guard.ping(reconnect=False)
            guard_failure = {"failed": False}
        except pymysql.MySQLError as error:
            guard_failure = {"failed": True, "code": error.args[0]}
        require(guard_failure["failed"], "lost guard heartbeat unexpectedly succeeded")
        successful(writer, "ROLLBACK")
        unchanged = successful(observer, "SELECT id, note FROM `" + names[4] + "`")
        require(unchanged["rows"] == ((2, "replacement"),), unchanged)
        report["cases"]["guardLostBeforeWriter"] = {"cachedStatusBeforeLoss": cached_before_loss, "cachedStatusAfterLoss": status(guard), "clientHandleOpenAfterTransportLoss": handle_open_after_loss, "replacementAfterLoss": replaced, "writerCanReadReplacement": new_read, "originalGuardRoundtrip": guard_failure, "noDmlSubmitted": True, "replacementRow": unchanged["rows"]}
        writer.close()

        create(observer, names[5])
        guard, writer = connect(), connect()
        guard_table(guard, names[5])
        writer_read = guard_table(writer, names[5])
        break_transport(guard)
        blocked = sql(observer, "DROP TABLE `" + names[5] + "`")
        timeout(blocked)
        successful(writer, "ROLLBACK")
        released = successful(observer, "DROP TABLE `" + names[5] + "`")
        report["cases"]["guardLostAfterWriterLock"] = {"writerRead": writer_read, "ddlBlockedByWriter": blocked, "ddlAfterWriterRollback": released}
        writer.close()
        report["passed"] = True
    except BaseException as error:
        report["passed"] = False
        report["failure"] = {"type": type(error).__name__, "message": str(error)}
    finally:
        errors = list(setup_close_errors)
        setup_errors_recorded = len(setup_close_errors)

        def record(phase, error, **details):
            errors.append({"phase": phase, "type": type(error).__name__, "message": str(error), **details})

        def close_owned(connection, label):
            try:
                if connection.open:
                    connection.close()
            except BaseException as error:
                record("connectionClose", error, connection=label)

        # Release guards first. A pending worker owns its connection until joined;
        # do not send COM_QUIT concurrently with its in-flight query.
        busy_connections = {id(connection) for thread, _results, connection in jobs if thread.is_alive()}
        for index, connection in enumerate(opened):
            if id(connection) not in busy_connections:
                close_owned(connection, index)
        worker_states = []
        for index, (thread, results, connection) in enumerate(jobs):
            try:
                thread.join(9)
                if thread.is_alive():
                    errors.append({"phase": "workerJoin", "worker": index, "message": "Worker exceeded bounded join; interrupting its owned transport"})
                    try:
                        connection._sock.shutdown(socket.SHUT_RDWR)
                    except BaseException as error:
                        record("workerTransportShutdown", error, worker=index)
                    thread.join(1)
                joined = not thread.is_alive()
                worker_states.append({"worker": index, "joined": joined})
                if not joined:
                    errors.append({"phase": "workerJoin", "worker": index, "message": "Worker remained alive after transport interruption"})
                    # Avoid protocol writes from two threads; close only the socket.
                    try:
                        connection._sock.close()
                    except BaseException as error:
                        record("workerTransportClose", error, worker=index)
                else:
                    close_owned(connection, "worker-" + str(index))
                while not results.empty():
                    result = results.get_nowait()
                    if not result["ok"]:
                        errors.append({"phase": "unconsumedWorkerResult", "worker": index, "result": result})
            except BaseException as error:
                record("workerCleanup", error, worker=index)

        cleanup = None
        drops = []
        remaining = None
        try:
            cleanup = connect(track=False)
            for name in names:
                try:
                    result = sql(cleanup, "DROP TABLE IF EXISTS `" + name + "`")
                    drops.append({"table": name, "result": result})
                    if not result["ok"]:
                        errors.append({"phase": "drop", "table": name, "result": result})
                except BaseException as error:
                    record("drop", error, table=name)
                    drops.append({"table": name, "exception": str(error)})
            # Verify actual catalog absence even if one or more DROP attempts failed.
            try:
                verification = sql(cleanup, "SELECT TABLE_NAME FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_SCHEMA = 'joinery' AND TABLE_NAME IN (" + ",".join(["%s"] * len(names)) + ")", names)
                if verification["ok"]:
                    remaining = verification["rows"]
                    if remaining:
                        errors.append({"phase": "verifyAbsence", "remainingTables": remaining})
                else:
                    errors.append({"phase": "verifyAbsence", "result": verification})
            except BaseException as error:
                record("verifyAbsence", error)
        except BaseException as error:
            record("cleanupConnect", error)
            for name in names:
                drops.append({"table": name, "attempted": False, "reason": "Cleanup connection unavailable"})
        finally:
            if cleanup is not None:
                close_owned(cleanup, "cleanup")
        # Include any setup-close error from the untracked cleanup connection.
        errors.extend(setup_close_errors[setup_errors_recorded:])
        report["cleanup"] = {"verified": remaining is not None and not remaining, "remainingTables": remaining, "drops": drops, "workers": worker_states, "errors": errors}
        if errors or not report["cleanup"]["verified"]:
            report["passed"] = False
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run-disposable-fixtures", action="store_true", required=True)
    parser.add_argument("--output", default="/tmp/joinery-mysql-mdl-report.json")
    arguments = parser.parse_args()
    report = {"kind": "standalone-fixture-protocol-design-evidence", "applicationSafetyQualified": False, "engineReports": []}
    for engine, port, dictionary in FIXTURES:
        report["engineReports"].append(run_engine(engine, port, dictionary))
    report["passed"] = all(entry["passed"] and entry["cleanup"]["verified"] for entry in report["engineReports"])
    rendered = json.dumps(report, indent=2, sort_keys=True)
    Path(arguments.output).write_text(rendered + "\n")
    print(rendered)
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
