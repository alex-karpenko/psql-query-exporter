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

-- for multi_labels queries
create table multi_labels (
    int_field int8,
    float_field float8,
    name varchar(255)
);
insert into multi_labels (int_field, float_field, name) values (1, 1.1, 'John');
insert into multi_labels (int_field, float_field, name) values (2, 2.2, 'Jane');
insert into multi_labels (int_field, float_field, name) values (3, 3.3, 'Jack');

create table multi_labels_to_drop (
    int_field int8,
    float_field float8,
    name varchar(255)
);
insert into multi_labels_to_drop (int_field, float_field, name) values (11, 11.01, 'John-11');
insert into multi_labels_to_drop (int_field, float_field, name) values (12, 12.02, 'Jane-12');
insert into multi_labels_to_drop (int_field, float_field, name) values (13, 13.03, 'Jack-13');
