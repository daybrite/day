// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
#include <QApplication>
#include <QTreeWidget>
#include <QCoreApplication>
#include <QEvent>
#include <cassert>
#include <cstdint>
extern "C" {
void day_qt_open_file(const char *) {}
void *day_qt_tree_new(uint64_t,double,double,int,int,void(*)(uint64_t,uint64_t,int),void(*)(uint64_t,const uint64_t*,int),void(*)(void*));
void day_qt_tree_begin(void*);
void day_qt_tree_add(void*,uint64_t,uint64_t,int,const char*,int,int,int);
void day_qt_tree_end(void*);
void *day_qt_tree_cell(void*,uint64_t);
int day_qt_tree_frame(void*,uint64_t,double*);
void day_qt_tree_expand(void*,uint64_t,int);
void day_qt_tree_select(void*,const uint64_t*,int);
}
static int changes=0;
static uint64_t changed=0;
static constexpr uint64_t group=0xf123456789abcdefULL;
static void expansion(uint64_t node,uint64_t token,int) { assert(node==7); ++changes; changed=token; }
static void selection(uint64_t,const uint64_t*,int) {}
static void viewport(void*) {}
int main(int argc,char **argv) {
    QApplication app(argc,argv);
    auto *tree=static_cast<QTreeWidget*>(day_qt_tree_new(7,32,16,1,0,expansion,selection,viewport));
    tree->resize(300,300); tree->show();
    auto populate=[&](bool open) {
        day_qt_tree_begin(tree);
        day_qt_tree_add(tree,group,0,0,"Fixture group",1,1,open);
        day_qt_tree_add(tree,42,group,1,"Fixture child",0,0,0);
        day_qt_tree_end(tree); QCoreApplication::processEvents();
    };
    populate(true); assert(changes==0);
    auto *cell=static_cast<QWidget*>(day_qt_tree_cell(tree,42));
    double width=0; assert(day_qt_tree_frame(tree,42,&width)); assert(width>0);
    tree->topLevelItem(0)->setExpanded(false); // native event, not Day's synthetic expand
    assert(changes==1 && changed==group);
    assert(!day_qt_tree_frame(tree,42,&width)); assert(!cell->isVisible());
    day_qt_tree_expand(tree,group,1); assert(changes==1);
    assert(day_qt_tree_frame(tree,42,&width));
    day_qt_tree_select(tree,&group,1); assert(tree->selectedItems().isEmpty());
    uint64_t child=42; day_qt_tree_select(tree,&child,1); assert(tree->selectedItems().size()==1);
    populate(false); QCoreApplication::sendPostedEvents(nullptr,QEvent::DeferredDelete);
    assert(day_qt_tree_cell(tree,42)==cell); // model reload never deletes a Day anchor
    day_qt_tree_expand(tree,group,1); assert(changes==1);
    assert(day_qt_tree_frame(tree,42,&width));
    delete tree;
}
