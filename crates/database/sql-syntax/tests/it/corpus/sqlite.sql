-- Valid SQLite queries: the SQLite dialect parses them without an error and
-- sqlparser's SQLite dialect accepts them. Blocks are separated by blank
-- lines; a block that starts with `-- no-oracle` is valid by the manual but
-- not accepted by sqlparser 0.63, and is not given to it; `-- not-generic`:
-- not lexed the same way by the generic dialect (parameter forms).

select id from tracks limit 10

select id from tracks limit 10 offset 5

select id from tracks limit 5, 10

select * from [Track Data]

select `name` from `albums`

select "name" from "albums"

select id from tracks where name glob 'A*'

-- no-oracle
select id from tracks where name not glob '*x*'

-- no-oracle
select id from tracks where name regexp '^a'

-- no-oracle
select id from tracks where name match 'love'

select id from tracks where name like 'a%' escape '\'

-- no-oracle
select id from tracks where composer isnull

select id from tracks where composer notnull

-- no-oracle
select id from tracks where composer not null

select id from tracks where composer is not null

-- no-oracle
select id from tracks where a is b

-- no-oracle
select id from tracks where a is not b

select id from tracks where a == b

select id from tracks where milliseconds in (select milliseconds from favourites)

-- no-oracle
select id from tracks where genre_id in genres

select typeof(x), quote(x), hex(x), length(x) from t

select ifnull(composer, 'unknown'), iif(milliseconds > 1000, 'long', 'short') from tracks

select group_concat(name, ', ') from tracks

select group_concat(distinct composer) from tracks

select count(*) filter (where milliseconds > 1000) from tracks

select date('now'), time('now'), datetime('now', '-1 day'), strftime('%Y', 'now')

select json_extract(doc, '$.a'), doc -> '$.a', doc ->> '$.b' from docs

select * from json_each('[1, 2, 3]')

select key, value from json_tree('{"a": 1}')

select cast(x as integer), cast(y as real), cast(z as text), cast(w as blob) from t

select x'0A0B', 0x1F

select name collate nocase from tracks order by name collate nocase

select id from tracks order by name desc nulls last

select id, row_number() over (order by milliseconds) from tracks

select id, sum(bytes) over (partition by album_id order by id rows between unbounded preceding and current row) from tracks

select id, avg(bytes) over win from tracks window win as (partition by album_id)

-- no-oracle
select id, sum(bytes) over (order by id groups 1 preceding exclude ties) from tracks

with recursive cnt(x) as (select 1 union all select x + 1 from cnt where x < 10) select x from cnt

with a as (select 1) select * from a

-- no-oracle
with a as materialized (select 1) select * from a

-- no-oracle
with a as not materialized (select 1) select * from a

select * from tracks t join albums a on a.id = t.album_id left join artists r on r.id = a.artist_id

select * from tracks natural join albums

select * from tracks join albums using (album_id)

select * from tracks cross join albums

select * from tracks join albums

-- not-generic
select ?1, ?2, :name, @var, $param from t

select * from (select id from tracks) as sub

select total(bytes), sum(bytes) from tracks

select id from tracks where milliseconds between 1000 and 2000 and name like 'S%'

select case when bytes > 100 then 'big' else 'small' end from tracks

select distinct album_id from tracks order by 1 limit 3

select a.id from tracks a where exists (select 1 from albums b where b.id = a.album_id)

select album_id, count(*) from tracks group by album_id having count(*) > 10

select album_id from tracks union select id from albums order by 1 limit 5

select abs(-1), max(1, 2), min(3, 4), random(), round(1.5)

select substr(name, 2, 3), instr(name, 'a'), trim(name), ltrim(name, 'x') from tracks

select printf('%d items', count(*)) from tracks

select * from pragma_table_info('tracks')

select id from tracks where rowid = 1

select changes(), last_insert_rowid(), total_changes()

select 1 where 1

select 'a' || 'b' || 'c'

values (1, 2), (3, 4)

-- no-oracle
select * from tracks indexed by idx_tracks_name where name = 'x'

-- no-oracle
select * from tracks not indexed where name = 'x'

select coalesce(a, b, c) from t

select id from tracks where not (bytes > 10 or milliseconds < 5)

select likelihood(x, 0.5), unlikely(y) from t

select name from sqlite_master where type = 'table'

select id from tracks order by random() limit 1

select album_id, max(milliseconds) from tracks group by album_id order by 2 desc

select id from tracks where name like '%love%' and composer is not null

select upper(name), lower(name) from artists

select id, lag(bytes) over (order by id), lead(bytes, 2) over (order by id) from tracks

select id, ntile(4) over (order by bytes) from tracks

select a.name, count(t.id) from albums a left join tracks t on t.album_id = a.id group by a.id

select * from tracks where album_id in (1, 2, 3) order by name

select julianday('now') - julianday(created) from orders

select unicode('a'), char(97), zeroblob(4)

select id from tracks where exists (select 1 from playlist_track p where p.track_id = tracks.id)

select id from tracks except select track_id from playlist_track

select id from tracks intersect select track_id from playlist_track

select coalesce(composer, 'n/a') as c from tracks order by c

select min(milliseconds), max(milliseconds), avg(milliseconds) from tracks
