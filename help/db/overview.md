---
id: db-overview
title: Databases
summary: Browse a project's databases, read and edit table data, and run SQL — PostgreSQL, MySQL, SQLite and SQL Server, with a read-only guard.
keywords: [database, db, sql, postgres, mysql, sqlite, sql server, table, query, connection, password]
context: [rail.db, panel.ubiq.db.explorer, panel.ubiq.db.table, panel.ubiq.db.sql]
targets: [ui.rail.mode.db]
order: 10
status: current
related: [workbench-modes, workbench-rail, kb-overview]
---

## Turning it on

**DB** is off until a project asks for it. Open the project's settings, go to **General**, and light
the **DB** tile under Modes; the mode then appears in the rail directly after IDE. Each project
decides for itself.

## Connections

A connection is a name and where a database lives: a host, port, database, user and password for
PostgreSQL, MySQL/MariaDB and SQL Server, or a file for SQLite. Add one with **+** in the explorer's
header, or from the project settings' **Databases** section, which lists every connection with
Test, Edit and Remove.

- **Paste a connection string** into the form and press Parse; it fills the fields for you.
- **Test** connects and reports the server's version, without saving anything.
- **Read-only** makes every tab on that connection refuse to write.
- For SQLite, pick a file on the machine the project lives on, or choose **New database…** to
  create an empty one.

The connection list is part of the project: for a project that keeps its data inside its own
folder, it is committed with it and a teammate sees the same connections. **Passwords are never in
that list.** A saved password is encrypted, kept on this machine only, and opened only by the host;
the form never shows it back, only whether one is `saved`. Leave the field empty to keep it, type to
replace it, or press *Forget password*. If a password is missing — a reset keychain, a copied
configuration — Ubiq asks for it when you connect; nothing breaks.

## The explorer

The explorer lists connections and, under each, its databases, schemas, tables and views, and their
columns. Each level loads when you first open it. The filter above the tree narrows what is already
loaded by name. Double-click a table to open it; right-click any row for its menu — connect, new SQL
editor, open the data, refresh, copy a name, edit or remove a connection.

## A table tab

A table opens in the centre. Type a **WHERE** and an **ORDER BY** to narrow it, page through the
rows, and read the count (an estimate until the exact one arrives).

- **Edit a cell** by double-clicking it, or press **Enter** or **F2**; or switch to the **Form**
  layout and edit a whole row at once. Backspace in an empty field makes a cell NULL.
- Changes stay **pending**: edited cells are bold and their rows tinted. The **SQL preview** shows
  exactly what Apply will run; Apply sends the batch in one transaction, and rolls it all back if
  one statement fails. Discard drops it.
- The lock makes a tab read-only. A view is always read-only.

## SQL tabs

A SQL tab opens along the bottom; open as many as you like, each on its own connection and
database.

- **Run** (⌘↵ or Ctrl↵) runs the selection, or the statement the cursor is in — the toolbar names
  it. **Run all** (⌘⇧↵ or Ctrl⇧↵) runs every statement, one result tab each.
- **Stop** (⌘. or Ctrl.) cancels what is running; a timer shows how long it has taken.
- **Explain** shows the plan as a tree with the costliest step marked; **Explain analyze** runs the
  statement for real to measure it, so it asks first.
- A statement that can change data asks before it runs.

## Read-only means read-only

A read-only connection or tab cannot write, and Ubiq does not take the editor's word for it: the
host checks every statement before sending it and also tells the database to refuse a write. The
editor underlines a statement it knows will be refused. A result is cut after a thousand rows and
says so.

If a tab says a database is **unavailable**, the host you are connected to cannot open database
connections.
