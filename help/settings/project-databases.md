---
id: settings-project-databases
title: Project databases
summary: The Databases section of Project settings — the connections this project's databases are reached by, and the form that adds one.
keywords: [database, connection, postgresql, mysql, mariadb, sqlite, sql server, read-only, password, connection string, agents]
order: 91
status: current
related: [settings-project, db-overview, guides-query-a-database]
---

**Databases** lists the connections this project reaches databases by. Each connection belongs to the
project, and the **DB** mode of the rail opens them: see [Databases](db-overview). This section is
where you add, test, change and remove them. It appears once the folder is kept as a project.

## The parts

| Part | What it does |
|---|---|
| A connection row | Its name, its engine and where it points, a **read-only** badge, and what is held for the password: **saved**, **not saved**, **session** or **needs password** |
| **Test** | Tries the connection and says **Connected**, with the server version, or why not |
| **Edit**, **Remove** | Reopens the form; **Remove** asks **Remove this connection?** in the row, with **Keep** and **Remove** |
| **Add connection** | Opens the form below |
| **Name**, **Engine** | The connection's name, and **PostgreSQL**, **MySQL** and **MariaDB**, **SQLite** or **SQL Server** |
| **Connection string** | Paste an address and **Parse** fills the fields below |
| **Host**, **Port**, **Database** | Where the server is; empty host means this machine, empty port the engine's own |
| **Database file** | SQLite: **Browse…** for an existing file or **New database…** to create an empty one |
| **Authentication**, **User**, **Password** | Sign-in. **Integrated** sends no user or password; a saved password is never shown, and leaving it empty keeps it |
| **Remember on this machine**, **Forget password** | Whether the password is kept, and a way to throw it away |
| **Encryption** | **Off**, **Prefer**, **Require** or **Verify** |
| **Read-only** | The connection refuses every statement that writes, whatever a tab says |
| **Agents**, **Default** | What an agent may do through it: **None**, **Read-only** or **Read-write**; **Default for agents** makes it the one an agent gets when it names none, one per project |
| **Description** | What the connection is for, as an agent reads it |

## What you can do here

- Run a query once a connection exists: [Query a database](guides-query-a-database).
- Show or hide the DB mode: **General**, then **Modes** in [Project settings](settings-project).

## When something goes wrong

- **A warning says this install cannot keep passwords.** A password you type is held until Ubiq
  closes, then asked for again.
- **A row says needs password.** Press **Edit**, enter it, and **Save**.
- **Read-write is dimmed.** A read-only connection cannot give an agent write access; turn off
  **Read-only** first.
- **Default for agents is dimmed.** Give the connection some agent access first.
