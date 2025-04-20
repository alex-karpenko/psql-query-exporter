-- to test basic db access rouitines
create table basics (
    id int8,
    name varchar(255)
);
insert into basics (id, name) values (1, 'John');
insert into basics (id, name) values (2, 'Jane');
insert into basics (id, name) values (3, 'Jack');

-- for single value queries
create table single (
    id int8,
    name varchar(255)
);
insert into single (id, name) values (1, 'John');
insert into single (id, name) values (2, 'Jane');
insert into single (id, name) values (3, 'Jack');

create table single_to_drop (
    id int8,
    name varchar(255)
);
