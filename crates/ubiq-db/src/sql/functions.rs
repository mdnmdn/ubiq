//! The read-only function allowlist — one readable table per dialect.
//!
//! A function call is allowed only if its (lower-cased, unqualified) name is listed here. The lists
//! hold **known-pure** functions: they compute a value from their arguments and the clock/session
//! and change nothing. Everything else — sequences (`nextval`, `setval`), sleeps (`pg_sleep`,
//! `sleep`, `benchmark`), locks (`get_lock`), file and network access (`pg_read_file`,
//! `lo_import`, `dblink`, `load_file`, `openrowset`, `load_extension`), session control
//! (`pg_terminate_backend`, `set_config`) and every user-defined function — is absent on purpose.
//! Adding a name is a conscious act: check it cannot write, lock, wait or reach outside the database.
//!
//! Each table is whitespace-separated lower-case names. `COMMON` applies to every dialect;
//! `Generic` accepts the union of all tables.

use super::Dialect;

/// Standard SQL and the functions every engine spells the same way.
const COMMON: &str = "
    count sum avg min max any_value every bit_and bit_or bit_xor
    stddev stddev_pop stddev_samp variance var_pop var_samp corr covar_pop covar_samp
    percentile_cont percentile_disc mode
    row_number rank dense_rank percent_rank cume_dist ntile lag lead first_value last_value nth_value
    coalesce nullif greatest least
    abs ceil ceiling floor round mod power sqrt exp ln log log10 sign pi
    sin cos tan asin acos atan atan2 degrees radians
    lower upper length char_length character_length octet_length trim ltrim rtrim substring
    replace concat position reverse repeat left right
    current_date current_time current_timestamp localtime localtimestamp
    current_user session_user user
    cast extract
    json_object json_array json_extract json_valid json_type json_quote json_length
";

/// PostgreSQL. `pg_catalog.`-qualified calls are accepted too.
const POSTGRES: &str = "
    string_agg array_agg bool_and bool_or json_agg jsonb_agg json_object_agg jsonb_object_agg
    xmlagg
    div trunc random cbrt log2 gcd lcm width_bucket scale min_scale
    btrim initcap lpad rpad strpos split_part translate ascii chr md5 sha224 sha256 sha384 sha512
    to_hex encode decode format quote_ident quote_literal quote_nullable left right overlay
    regexp_replace regexp_match regexp_matches regexp_split_to_array regexp_split_to_table
    regexp_count regexp_instr regexp_like regexp_substr string_to_array array_to_string bit_length
    now clock_timestamp statement_timestamp transaction_timestamp timeofday
    date_trunc date_part date_bin age make_date make_time make_timestamp make_timestamptz
    make_interval justify_days justify_hours justify_interval isfinite timezone
    to_char to_date to_timestamp to_number
    generate_series generate_subscripts unnest
    array_length array_upper array_lower array_dims array_ndims cardinality array_position
    array_positions array_append array_prepend array_cat array_remove array_replace array_fill
    json_build_object json_build_array jsonb_build_object jsonb_build_array to_json to_jsonb
    row_to_json json_typeof jsonb_typeof json_array_length jsonb_array_length json_each jsonb_each
    json_each_text jsonb_each_text json_array_elements jsonb_array_elements
    json_array_elements_text jsonb_array_elements_text json_object_keys jsonb_object_keys
    json_extract_path json_extract_path_text jsonb_extract_path jsonb_extract_path_text
    jsonb_pretty jsonb_set jsonb_insert jsonb_strip_nulls json_strip_nulls jsonb_path_query
    jsonb_path_query_array jsonb_path_query_first jsonb_path_exists jsonb_contains
    json_populate_record jsonb_populate_record
    to_tsvector to_tsquery plainto_tsquery websearch_to_tsquery ts_rank ts_rank_cd ts_headline
    version current_database current_schema current_schemas current_catalog current_setting
    pg_typeof format_type to_regclass to_regtype to_regproc pg_table_is_visible pg_type_is_visible
    pg_get_viewdef pg_get_indexdef pg_get_constraintdef pg_get_expr pg_get_userbyid
    pg_table_size pg_total_relation_size pg_relation_size pg_indexes_size pg_database_size
    pg_size_pretty pg_size_bytes pg_relation_filepath pg_is_in_recovery pg_postmaster_start_time
    obj_description col_description shobj_description
    has_table_privilege has_schema_privilege has_database_privilege has_column_privilege
    has_sequence_privilege has_function_privilege
    inet_client_addr inet_server_addr host network netmask masklen family abbrev text
";

/// MySQL and MariaDB. Backtick-quoted function names are refused outright (they would name a
/// stored function, not a built-in).
const MYSQL: &str = "
    group_concat json_arrayagg json_objectagg bit_count std stddev_samp
    truncate rand div crc32 conv hex unhex ln log2 cot
    lcase ucase concat_ws substr mid instr locate lpad rpad ltrim format insert space strcmp
    ascii ord char bin oct elt field find_in_set substring_index soundex quote to_base64
    from_base64 md5 sha sha1 sha2 export_set make_set regexp_replace regexp_like regexp_substr
    regexp_instr
    ifnull if isnull
    now sysdate curdate curtime current_date utc_date utc_time utc_timestamp unix_timestamp
    from_unixtime date time timestamp year month day hour minute second microsecond week weekday
    weekofyear dayofweek dayofmonth dayofyear dayname monthname quarter yearweek last_day
    date_add date_sub adddate subdate addtime subtime datediff timediff timestampdiff
    timestampadd date_format time_format str_to_date to_days from_days to_seconds sec_to_time
    time_to_sec makedate maketime period_add period_diff get_format convert_tz
    json_unquote json_contains json_contains_path json_keys json_depth json_search json_set
    json_insert json_replace json_remove json_merge_patch json_merge_preserve json_overlaps
    json_pretty json_table
    database schema version connection_id found_rows row_count
    st_astext st_asgeojson st_distance st_x st_y
";

/// SQLite. The `pragma_*` table-valued functions only read.
const SQLITE: &str = "
    total group_concat
    ifnull iif nullif likely unlikely likelihood
    random randomblob zeroblob hex unhex quote printf format char unicode instr substr
    ltrim rtrim lower upper glob like typeof
    date time datetime julianday unixepoch strftime timediff
    json json_array_length json_each json_tree json_group_array json_group_object json_set
    json_insert json_replace json_remove json_patch json_extract json_type json_valid
    sqlite_version sqlite_source_id last_insert_rowid
    pragma_table_info pragma_table_xinfo pragma_table_list pragma_index_list pragma_index_info
    pragma_index_xinfo pragma_foreign_key_list pragma_database_list pragma_function_list
    pragma_collation_list
";

/// SQL Server (T-SQL). No `OPENROWSET`/`OPENQUERY`/`OPENDATASOURCE`, no `xp_*`/`sp_*`.
const MSSQL: &str = "
    string_agg count_big checksum_agg stdev stdevp var varp approx_count_distinct
    ceiling square rand
    len datalength charindex patindex stuff substring ltrim rtrim lower upper replicate
    reverse space str char nchar ascii unicode soundex difference format concat concat_ws
    string_split translate trim quotename isnull iif choose
    getdate getutcdate sysdatetime sysutcdatetime sysdatetimeoffset current_timestamp
    dateadd datediff datediff_big datepart datename day month year eomonth
    datefromparts datetimefromparts datetime2fromparts timefromparts smalldatetimefromparts
    isdate switchoffset todatetimeoffset
    convert try_convert try_cast parse try_parse
    isjson json_value json_query json_modify openjson
    newid scope_identity ident_current ident_incr ident_seed
    db_name db_id schema_name schema_id object_id object_name object_schema_name col_name
    columnproperty objectproperty serverproperty suser_name suser_sname user_name host_name
    app_name has_perms_by_name
";

/// Whether the call `name` is on the allowlist for `dialect`. `name` is lower-case, unqualified.
pub(super) fn is_allowed(dialect: Dialect, name: &str) -> bool {
    let listed = |table: &str| table.split_whitespace().any(|w| w == name);
    listed(COMMON)
        || match dialect {
            Dialect::Postgres => listed(POSTGRES),
            Dialect::MySql => listed(MYSQL),
            Dialect::Sqlite => listed(SQLITE),
            Dialect::MsSql => listed(MSSQL),
            Dialect::Generic => {
                listed(POSTGRES) || listed(MYSQL) || listed(SQLITE) || listed(MSSQL)
            }
        }
}

/// SQL Server table hints that only influence the plan or relax isolation. Lock-taking hints
/// (`UPDLOCK`, `XLOCK`, `HOLDLOCK`, `SERIALIZABLE`, `TABLOCK`, `TABLOCKX`, …) are absent.
const TABLE_HINTS: &str = "
    nolock readuncommitted readcommitted readpast snapshot nowait forceseek forcescan noexpand
";

pub(super) fn is_allowed_table_hint(name: &str) -> bool {
    let lower = name.to_lowercase();
    TABLE_HINTS.split_whitespace().any(|w| w == lower)
}

/// SQLite pragmas that only report. The first list takes an optional single argument
/// (`PRAGMA table_info(t)`); the second takes none, and with `=` or `(…)` it would set a value.
pub(super) const PRAGMAS_WITH_ARG: &str = "
    table_info table_xinfo table_list index_list index_info index_xinfo foreign_key_list
";
pub(super) const PRAGMAS_NO_ARG: &str = "
    database_list collation_list function_list module_list pragma_list compile_options
    foreign_key_check integrity_check quick_check user_version schema_version page_count
    page_size encoding
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dangerous_names_are_absent_everywhere() {
        for name in [
            "nextval",
            "setval",
            "pg_sleep",
            "pg_terminate_backend",
            "pg_read_file",
            "lo_import",
            "dblink",
            "sleep",
            "benchmark",
            "load_file",
            "get_lock",
            "xp_cmdshell",
            "openrowset",
            "load_extension",
            "set_config",
        ] {
            assert!(!is_allowed(Dialect::Generic, name), "{name}");
        }
    }
}
