---
id: db-overview
title: Databases
summary: DB mode — browse a project's databases, read and edit table data, and run SQL on PostgreSQL, MySQL, SQLite and SQL Server.
keywords: [database, db, sql, postgres, mysql, mariadb, sqlite, sql server, table, query, connection, password, read-only]
context: [rail.db, panel.ubiq.db.explorer, panel.ubiq.db.table, panel.ubiq.db.sql]
targets: [ui.rail.mode.db]
order: 10
status: current
related: [guides-query-a-database, workbench-modes, workbench-rail, kb-overview]
---

**DB** on the rail holds a project's database connections. The **Databases** panel on the left lists
them and their contents, a table opens in the centre, and SQL editors open beside it. DB is off until
a project asks for it: open the project's settings, go to **General**, and light the **DB** tile under
**Modes**. Each project decides for itself, and the mode then appears after IDE.

## The parts

| Part | What it does |
|---|---|
| **+** (**Add connection**) in the explorer header | Opens the connection form |
| The gear (**Database settings**) | Opens the project's settings on **Databases**, which lists every connection with **Test**, **Edit** and **Remove**; see [Project databases](settings-project-databases) |
| The filter above the tree | Narrows what is already loaded, by name |
| The tree | Connections, then databases, schemas, tables and views, then columns; each level loads when first opened |
| A table row | Double-click to open; its approximate row count sits at the right |
| **WHERE**, **ORDER BY**, **Filter** | Narrow a table tab; page with the previous and next buttons |
| Edited cells and the pending count | Edited cells are bold and their rows tinted until you **Apply** or discard |
| **SQL preview** | The statements **Apply** will run |
| **Inline**, **Form** | Edit in the grid, or one whole row at a time in a side form |
| **Run**, **Run all**, **Explain**, **Analyze**, **Stop** | The SQL tab's toolbar |
| **READ-ONLY** | Marks a tab that cannot write |

Right-click a row for its menu. A connection that is not connected offers **Connect**, **Edit
connection…** and **Remove**; once connected it offers **New SQL editor**, **Refresh**,
**Disconnect**, the DBML rows, **Edit connection…** and **Remove**. A table or view offers **Open
data**, **Open in SQL editor**, **Copy name**, **Refresh** and the DBML rows; **DBML: Copy to
clipboard** puts the schema on the clipboard, and **DBML: Open in new file** opens it as a file.

## What you can do here

- Add a connection: fill in the form, or paste a connection string and press **Parse**. **Test**
  connects and reports the server version without saving. For SQLite pick a file, or choose
  **New database…**. Tick **Read-only** to make every tab on the connection refuse writes. On the
  empty screen, **Add connection** opens the project's settings instead, where the same form is.
- Decide what agents may do with a connection: in the form, **Agents** is **None**, **Read-only** or
  **Read-write**; **Read-write** is dimmed on a read-only connection. **Default for agents** marks
  the connection an agent gets when it names none; one per project, and it needs some access.
- Edit a table: double-click a cell, or press **Enter** or **F2**; **Backspace** in an empty field makes
  the cell NULL. **Apply** sends all pending changes as one batch and undoes them all if one fails.
- Run SQL: **Run** (**⌘Enter** / **Ctrl+Enter**) runs the selection or the statement the cursor is in;
  **Run all** (**⌘⇧Enter** / **Ctrl+Shift+Enter**) runs every statement, one result tab each; **Stop**
  (**⌘.** / **Ctrl+.**) cancels. **Analyze** runs the statement for real to measure it, so it asks first.
- Share the connections: they are saved with the project, and a teammate sees the same list. The
  walk-through is [Query a database](guides-query-a-database).

## When something goes wrong

- **A password is missing.** Passwords are never saved with the project, only on this machine. Ubiq
  asks when you connect; answer with **Remember on this machine** to keep it. The form shows
  whether one is `saved`, and **Forget password** clears it.
- **A connection says `failed`.** Use **Retry** on its row, or **Test** it in the form.
- **A statement is refused on a read-only tab.** Ubiq checks every statement before sending it, and
  underlines the one it will refuse.
- **A statement that can change data asks first**, and **Run anyway** goes ahead.
- **Results stop at a thousand rows** and say so; narrow the query.
- **A table with no primary key** carries a badge: edits match on every column.
- **Edits are pending.** Paging, refresh and filtering wait until you apply or discard them.
